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

use crate::extract::table::ruled::rule_breaks;
use crate::limits::Limits;
use crate::region::Bounds;
use crate::segment::drawn::{Line, SNAP, merged};
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
    /// The text of the piece is a whole number (see [`reads_as_number`]): a value that may
    /// also name a column (a year, a number of a column).
    pub number: bool,
    /// The piece stands in for a drawn box (the index of the box): its text is read whole
    /// where it stands, and `rows` counts its rows.
    pub boxed: Option<usize>,
    /// The rows of the box the piece stands in for; 1 for any other piece.
    pub rows: usize,
    /// The white space at the end of the extent, after the last glyph with ink: the blanks
    /// a print run pads a field with up to the next one, the space set after a word. 0 for
    /// a piece that ends with ink, and for a piece of white space only. The extent with
    /// its blanks says where the producer went on in one string (two pieces that touch
    /// leave no band of white space between them); where the piece stands at a gutter is
    /// asked of its ink (see [`Piece::ink_end`]).
    pub trail: f32,
}

impl Piece {
    /// Where the ink of the piece ends: its extent less the white space after its last
    /// glyph with ink.
    pub fn ink_end(&self) -> f32 {
        self.bounds.x1 - self.trail
    }
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
    /// labels of fields side by side, the parts of a row kept whole at a gutter.
    pub sides: Vec<usize>,
    /// Set when the cut stopped early. Nothing is dropped then: the pieces left stay in
    /// groups in their order. A caller that reports limits turns this into a note.
    pub short: Option<CutShort>,
    /// The indices, ascending, of the groups the depth bound or the stop kept whole where the
    /// cut would have looked into them further.
    pub kept: Vec<usize>,
    /// The indices of the pieces, ascending, that are a line of their own even when they
    /// share the row of the piece before or after them: a piece that went to a side with
    /// the ink of its row while its own ink reaches into a row of the other side (see
    /// `rows_whole`). It reads at its place in its row and is not said to be one line with
    /// the text beside it.
    pub breaks: Vec<usize>,
}

/// The joints of the rules `rules`, sorted by height then place: the height and the place
/// where a piece of a rule ends and the next begins (see [`rule_breaks`]). A rule drawn in one
/// piece, or handed over joined, has none. The rules are sorted by height once, and the
/// pieces of each height are looked at alone.
fn joints_of(rules: &[Line]) -> Vec<(f32, f32)> {
    if rules.len() < 2 {
        return Vec::new();
    }
    let mut sorted: Vec<Line> = rules.to_vec();
    sorted.sort_by(|a, b| a.at.total_cmp(&b.at));
    work::add(2 * sorted.len() as u64);
    let wide = (f32::MIN / 2.0, f32::MAX / 2.0);
    let mut out = Vec::new();
    let mut k = 0;
    while k < sorted.len() {
        // The pieces within a snap of the first one of this height.
        let y = sorted[k].at;
        let end = k + sorted[k..].partition_point(|r| r.at - y <= SNAP);
        if end - k > 1 {
            let breaks = rule_breaks(&sorted[k..end], wide, &[y]);
            out.extend(breaks.into_iter().map(|x| (y, x)));
        }
        k = end;
    }
    out
}

/// The pieces in reading order. `rules` are the horizontal drawn rules in the frame of the
/// pieces (`at` is the height), which part the bands of a gutter. `limits.max_cut_depth` bounds
/// the recursion above the fixed depth of the column search; `stop` is asked before each
/// region is looked at. Where either ends the cut, the pieces of the region at hand stay one
/// group in their order ([`Cut::short`] says so): no piece is dropped.
pub fn xy_cut(pieces: &[Piece], rules: &[Line], limits: &Limits, stop: &dyn Fn() -> bool) -> Cut {
    let ruled = Ruled {
        rules,
        joined: &[],
        joints: &[],
    };
    xy_cut_ruled(pieces, &ruled, limits, stop)
}

/// The horizontal rules of a page for the cut: the rules as the caller has them, the rules
/// that are drawn in pieces, each joined from its pieces, and the joints those pieces had
/// (the height and the place of each).
#[derive(Debug, Clone, Copy)]
pub struct Ruled<'a> {
    pub rules: &'a [Line],
    pub joined: &'a [Line],
    pub joints: &'a [(f32, f32)],
}

/// The flat and the upright lines among the drawn strokes `strokes`, as the cut and its
/// joints take them (`at` is the height of a flat line and the place of an upright one): each
/// stroke is its two end points `[x0, y0, x1, y1]` and whether it is a rectangle. A rectangle
/// gives its top and bottom as flat lines and its sides as upright ones. A line stroke is
/// flat when its ends differ in height by at most half a snap, upright when they differ in
/// place by at most that; a slanted stroke (hatching, a line struck through text) is
/// neither, and bounds nothing. The engine and a host that cuts its own text pass both sort
/// their strokes by this one rule, so both see the same rules and joints.
pub fn flat_and_upright(
    strokes: impl IntoIterator<Item = ([f32; 4], bool)>,
) -> (Vec<Line>, Vec<Line>) {
    let (mut flat, mut upright) = (Vec::new(), Vec::new());
    let mut n = 0u64;
    for ([ax, ay, bx, by], rect) in strokes {
        n += 1;
        let (x0, x1) = (ax.min(bx), ax.max(bx));
        let (y0, y1) = (ay.min(by), ay.max(by));
        let across = |at: f32| Line {
            at,
            from: x0,
            to: x1,
        };
        let up = |at: f32| Line {
            at,
            from: y0,
            to: y1,
        };
        if rect {
            flat.extend([across(y0), across(y1)]);
            upright.extend([up(x0), up(x1)]);
        } else if y1 - y0 <= SNAP / 2.0 {
            flat.push(across((y0 + y1) / 2.0));
        } else if x1 - x0 <= SNAP / 2.0 {
            upright.push(up((x0 + x1) / 2.0));
        }
    }
    work::add(n);
    (flat, upright)
}

/// The joints of the rules `rules` as drawn (see [`joints_of`]), and the rules with a joint,
/// each joined from its pieces with the join of the grid search. A joint that an upright line
/// of `uprights` (`at` is x, `from`..`to` along y) meets is none: it is a corner of a drawn
/// grid, whose cells its own lines bound; only a joint that no line marks stands for a column
/// bound that is not drawn. Made once for a page, as the cut is called for many parts of it.
pub fn joined(rules: &[Line], uprights: &[Line]) -> (Vec<Line>, Vec<(f32, f32)>) {
    work::add(rules.len() as u64);
    let mut joints = joints_of(rules);
    if !joints.is_empty() && !uprights.is_empty() {
        // The uprights by their place, so each joint looks at those near it only.
        work::add((uprights.len() + joints.len()) as u64);
        let mut by_x: Vec<&Line> = uprights.iter().collect();
        by_x.sort_by(|a, b| a.at.total_cmp(&b.at));
        joints.retain(|&(y, x)| {
            let from = by_x.partition_point(|u| u.at < x - SNAP);
            !by_x[from..]
                .iter()
                .take_while(|u| u.at <= x + SNAP)
                .any(|u| u.from <= y + SNAP && u.to >= y - SNAP)
        });
    }
    if joints.is_empty() {
        return (Vec::new(), joints);
    }
    // The joints are sorted by height: a rule has one when a joint lies within a snap of it.
    work::add(rules.len() as u64);
    let with_joint = |r: &Line| {
        let from = joints.partition_point(|j| j.0 < r.at - SNAP);
        joints.get(from).is_some_and(|j| j.0 <= r.at + SNAP)
    };
    let pieces: Vec<Line> = rules.iter().copied().filter(with_joint).collect();
    (merged(pieces), joints)
}

/// [`xy_cut`] with the joints of the rules: the joints of a rule drawn in pieces mark the
/// columns of the rows under it (see [`between_rules`]).
pub fn xy_cut_ruled(
    pieces: &[Piece],
    ruled: &Ruled<'_>,
    limits: &Limits,
    stop: &dyn Fn() -> bool,
) -> Cut {
    // The rules by height, once for the page: each region takes the stretch of its own
    // heights from them (see `Cutter::region`).
    let by_height = |rules: &[Line]| {
        let mut sorted = rules.to_vec();
        sorted.sort_by(|a, b| a.at.total_cmp(&b.at));
        sorted
    };
    let (rules, joined) = (by_height(ruled.rules), by_height(ruled.joined));
    let mut closing: Vec<f32> = rules.iter().chain(&joined).map(|r| r.at).collect();
    closing.sort_by(f32::total_cmp);
    work::add(2 * closing.len() as u64);
    let cutter = Cutter {
        p: pieces,
        rules,
        joined,
        closing,
        joints: ruled.joints,
        max_depth: limits.max_cut_depth as usize,
        stop,
    };
    let mut out = Out::default();
    cutter.region((0..pieces.len()).collect(), 0, &mut out);
    out.breaks.sort_unstable();
    out.breaks.dedup();
    Cut {
        groups: out.groups,
        splits: out.splits,
        sides: out.sides,
        short: out.short,
        kept: out.kept,
        breaks: out.breaks,
    }
}

/// Whether the cut prints its decisions to standard error: `REZZER_DEBUG_CUT` is set. For a
/// trace of a page that cannot be shared: the print holds line numbers, bounds and decisions
/// only, never text.
fn debug_cut() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("REZZER_DEBUG_CUT").is_some())
}

/// Prints one region of the cut (see [`debug_cut`]): its depth, its pieces by line (line id,
/// x range, baseline, size, whether it has ink) and the gutter found in it, if any.
fn debug_region(p: &[Piece], ids: &[usize], depth: usize, gutter: Option<(f32, f32, Why)>) {
    let mut sorted: Vec<usize> = ids.to_vec();
    sorted.sort_by_key(|&i| (p[i].line, p[i].bounds.x0 as i64));
    eprintln!(
        "CUT region depth {depth} pieces {} gutter {gutter:?}",
        ids.len()
    );
    for i in sorted {
        let q = &p[i];
        eprintln!(
            "CUT   line {} x {:.1}..{:.1} y {:.1} size {:.1} ink {} runs {}",
            q.line, q.bounds.x0, q.bounds.x1, q.baseline, q.size, q.inked, q.runs
        );
    }
}

#[derive(Default)]
struct Out {
    groups: Vec<Vec<usize>>,
    splits: Vec<Split>,
    sides: Vec<usize>,
    short: Option<CutShort>,
    kept: Vec<usize>,
    breaks: Vec<usize>,
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
    /// The rules as the caller has them, sorted by height.
    rules: Vec<Line>,
    /// The rules drawn in pieces, joined, sorted by height, and their joints: height and
    /// place (see [`joints_of`]).
    joined: Vec<Line>,
    /// The heights of all rules, sorted: any of them closes a block between rules.
    closing: Vec<f32>,
    joints: &'a [(f32, f32)],
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
        if debug_cut() {
            let found = found.as_ref().map(|(s, why)| (s.g0, s.g1, *why));
            debug_region(p, &ids, depth, found);
        }
        // A gutter that only a drawn box makes is taken after the bands of white space across:
        // text above or below the boxes is not part of their columns.
        if let Some((_, Why::Boxed)) = found {
            sort(&mut ids);
            let at = widest_gaps(p, &ids);
            if !at.is_empty() {
                if bound {
                    return self.keep(ids, out);
                }
                return self.bands(ids, &at, depth, out);
            }
        }
        if let Some((sides, why)) = found {
            let (g0, g1) = (sides.g0, sides.g1);
            if bound {
                return self.keep(ids, out);
            }
            sort(&mut ids);
            // Where each piece goes: to a side, alone across, or with its whole row.
            let (goes, _, breaks, _) = rows_whole(p, &ids, &sides);
            out.breaks.extend(breaks);
            // The pieces of the band that is open, sorted by row, each with where it goes:
            // to a side, or with its whole row.
            let mut band: Vec<(usize, Goes)> = Vec::new();
            // Reads the pieces `side` of one side of a band, sorted by row. A piece that
            // heads a part of the side (see [`Goes::HeadLeft`]) is a group of its own
            // between the part of the side over it and the part under it: it is not said
            // to be a line of either.
            let one_side = |side: Vec<(usize, Goes)>, out: &mut Out| {
                let mut flow: Vec<usize> = Vec::new();
                // Whether a part of the side is written: the next one stands under it.
                let mut under = false;
                for (i, go) in side {
                    if !matches!(go, Goes::HeadLeft | Goes::HeadRight) {
                        flow.push(i);
                        continue;
                    }
                    if !flow.is_empty() {
                        if under {
                            out.cut(Split::Band);
                        }
                        self.region(std::mem::take(&mut flow), depth + 1, out);
                        under = true;
                    }
                    if under {
                        out.cut(Split::Band);
                    }
                    out.group(vec![i]);
                    under = true;
                }
                if under && !flow.is_empty() {
                    out.cut(Split::Band);
                }
                self.region(flow, depth + 1, out);
            };
            // Reads the pieces `part` of a band on the two sides of the gutter, the left
            // side first.
            let two_sides = |part: &mut Vec<(usize, Goes)>, out: &mut Out| {
                if part.is_empty() {
                    return;
                }
                let (left, right): (Vec<_>, Vec<_>) = part
                    .drain(..)
                    .partition(|b| matches!(b.1, Goes::Left | Goes::HeadLeft));
                out.cut(Split::Gutter);
                one_side(left, out);
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
                one_side(right, out);
            };
            let flush = |band: &mut Vec<(usize, Goes)>, out: &mut Out| {
                if band.is_empty() {
                    return;
                }
                // The gutter holds in a band when most of its rows leave it white. A band
                // where most rows have ink in the gutter (one row of labels under a head
                // block, one of them set where the head block has its gutter) has no such
                // gutter of its own: it is looked at alone, and its rows are not parted on
                // the white space of the bands over and under it. The band that is the whole
                // region was looked at already.
                if band.len() < ids.len() {
                    work::add(band.len() as u64);
                    // The band is sorted by row: its rows with ink, and those of them with
                    // a piece that reaches into the gutter.
                    let (mut rows, mut filled) = (0usize, 0usize);
                    let mut last: Option<(u32, bool)> = None;
                    for q in band.iter().map(|&(i, _)| &p[i]).filter(|q| q.inked) {
                        let reaches = sides.reaches(q);
                        match &mut last {
                            Some((line, fills)) if *line == q.line => {
                                filled += usize::from(reaches && !*fills);
                                *fills |= reaches;
                            }
                            _ => {
                                rows += 1;
                                filled += usize::from(reaches);
                                last = Some((q.line, reaches));
                            }
                        }
                    }
                    if 2 * filled > rows {
                        out.cut(Split::Gutter);
                        let band: Vec<usize> = band.drain(..).map(|b| b.0).collect();
                        self.region(band, depth + 1, out);
                        return;
                    }
                }
                // The band is read on its two sides, down to each row that is kept whole.
                // Such a row is read at its height, from left to right. Its pieces of the
                // left side, the ones between and those of the right side are not said to
                // be one block: each part is a group of its own, on the row they share.
                let all = std::mem::take(band);
                let mut open: Vec<(usize, Goes)> = Vec::new();
                // The pieces that are read after the two sides of the open part.
                let mut after: Vec<usize> = Vec::new();
                let lone = |after: &mut Vec<usize>, out: &mut Out| {
                    for i in after.drain(..) {
                        out.cut(Split::Gutter);
                        out.group(vec![i]);
                    }
                };
                let mut k = 0;
                while k < all.len() {
                    if all[k].1 == Goes::After {
                        after.push(all[k].0);
                        k += 1;
                        continue;
                    }
                    if all[k].1 != Goes::Row {
                        open.push(all[k]);
                        k += 1;
                        continue;
                    }
                    two_sides(&mut open, out);
                    lone(&mut after, out);
                    let line = p[all[k].0].line;
                    let row: Vec<usize> = all[k..]
                        .iter()
                        .take_while(|b| b.1 == Goes::Row && p[b.0].line == line)
                        .map(|b| b.0)
                        .collect();
                    let stands = places(p, &row, &sides);
                    let mut part: Vec<usize> = Vec::new();
                    let mut last = None;
                    for (&i, &place) in row.iter().zip(&stands) {
                        let place = match place {
                            Place::Across => Place::Unsure,
                            place => place,
                        };
                        if last.is_some_and(|l| l != place) {
                            out.cut(Split::Gutter);
                            out.group(std::mem::take(&mut part));
                        }
                        last = Some(place);
                        part.push(i);
                        k += 1;
                    }
                    out.cut(Split::Gutter);
                    out.group(part);
                }
                two_sides(&mut open, out);
                lone(&mut after, out);
            };
            // A drawn rule across the gutter and most of the region parts the bands too: the
            // text above it is read before the text below it (a titled block before a grid).
            // Only a rule between the lowest and the highest piece of the region lies between
            // two of its pieces: the rules are sorted by height, and the region looks at the
            // stretch of its own heights, so its work grows with its own rules, not with
            // those of the page.
            work::add(ids.len() as u64);
            let ink = |of: fn(&Bounds) -> f32, pick: fn(f32, f32) -> f32, from: f32| {
                ids.iter().map(|&i| of(&p[i].bounds)).fold(from, pick)
            };
            let (lo, hi) = (
                ink(|b| b.x0, f32::min, f32::MAX),
                ink(|b| b.x1, f32::max, f32::MIN),
            );
            let (bottom, top) = (
                ink(|b| b.y0, f32::min, f32::MAX),
                ink(|b| b.y1, f32::max, f32::MIN),
            );
            fn within<T>(sorted: &[T], at: impl Fn(&T) -> f32, (bottom, top): (f32, f32)) -> &[T] {
                let from = sorted.partition_point(|r| at(r) < bottom);
                let to = sorted.partition_point(|r| at(r) <= top);
                &sorted[from..to.max(from)]
            }
            let wide = |rules: &[Line]| -> Vec<f32> {
                let own = within(rules, |r| r.at, (bottom, top));
                work::add(own.len() as u64);
                own.iter()
                    .filter(|r| r.from < g0 && r.to > g1 && r.to - r.from >= WIDE_RULE * (hi - lo))
                    .map(|r| r.at)
                    .collect()
            };
            // The bands are parted by the rules as the caller has them, not by the rules
            // joined from their pieces (review M4 of Task 31b): a rule drawn in pieces parts
            // the bands only where one piece alone is wide enough. Parting them by the joined
            // rules too was measured alone in fix round 2 and moved 9 public files, one of
            // them for the worse (a table of dates and events of the arxiv report kept as
            // laid out in parts). So a rule in one piece and the same rule in pieces may part
            // the bands of a page differently: a known break of the `split-rules` kind, kept
            // until the bands of a gutter are read by evidence of their own.
            let across = wide(&self.rules);
            // The text between two rules, one over the other, with rows that the gutter would
            // part and whose columns the joints of the upper rule mark, is a block of its own
            // (the rows of a table between the rule under its header and its closing rule):
            // it is looked at alone, so that only columns of its own part it, not the gutter
            // that the blocks over and under it make. Any rule of the page closes it.
            let between = if self.joints.is_empty() {
                Vec::new()
            } else {
                let pieced = wide(&self.joined);
                let closing = within(&self.closing, |&y| y, (bottom, top));
                work::add(closing.len() as u64);
                between_rules(p, &ids, (&pieced, closing), self.joints, &sides)
            };
            if debug_cut() {
                eprintln!(
                    "CUT   rules across {across:?} blocks between rules {between:?} (indices \
                     into the pieces of this region, sorted by line)"
                );
                eprintln!(
                    "CUT   ink of the sides ends and starts at {:?}, rows left {} right {}",
                    sides.edges(),
                    sides.left_rows.len(),
                    sides.right_rows.len()
                );
                for q in ids.iter().map(|&i| &p[i]).filter(|q| sides.reaches(q)) {
                    eprintln!(
                        "CUT   reaches in: line {} x {:.1}..{:.1} stands {:?}",
                        q.line,
                        q.bounds.x0,
                        q.bounds.x1,
                        sides.place(q)
                    );
                }
            }
            let mut next = 0;
            let mut low = f32::MAX;
            let mut at = 0;
            while at < ids.len() {
                let id = ids[at];
                // The blocks stand in the order of the pieces: the next one is looked up.
                while between.get(next).is_some_and(|b| b.0 < at) {
                    next += 1;
                }
                if let Some(&(_, end)) = between.get(next).filter(|b| b.0 == at) {
                    // A blank line right over the block (white space only) is no band of its
                    // own: it goes with the block under it.
                    let mut block: Vec<usize> = if band.iter().all(|&(i, _)| !p[i].inked) {
                        band.drain(..).map(|b| b.0).collect()
                    } else {
                        Vec::new()
                    };
                    flush(&mut band, out);
                    out.cut(Split::Rule);
                    block.extend_from_slice(&ids[at..end]);
                    self.region(block, depth + 1, out);
                    low = f32::MAX;
                    at = end;
                    continue;
                }
                at += 1;
                let ruled = across.iter().any(|&y| y < low && y > p[id].bounds.y1);
                if ruled {
                    flush(&mut band, out);
                    out.cut(Split::Rule);
                    low = f32::MAX;
                }
                match goes[at - 1] {
                    Goes::Across => {
                        flush(&mut band, out);
                        out.cut(Split::Gutter);
                        out.group(vec![id]);
                        low = f32::MAX;
                    }
                    goes => {
                        band.push((id, goes));
                        low = low.min(p[id].bounds.y0);
                    }
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

/// Whether `text` is a whole number: digits only, white space at its ends aside. A year over
/// a column of amounts or the number of a column is one; an amount with a decimal mark, a
/// share or a date is not.
pub fn reads_as_number(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty() && t.chars().all(|c| c.is_ascii_digit())
}

/// A piece reaches into a gutter when it reaches past its edge by more than this share of
/// a step of the bands (see [`BAND_STEP`]): half a point at 8 points.
const GUTTER_SLACK: f32 = 0.5 * BAND_STEP;

/// Where a piece stands at a gutter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// A line of the left side.
    Left,
    /// A line of the right side.
    Right,
    /// A piece across the gutter: a title, a note or a line of text over both sides.
    Across,
    /// The piece reaches into the gutter and neither a side nor a place across is proven:
    /// its row stays whole (see [`rows_whole`]).
    Unsure,
}

/// The outer edge of the ink of one row on one side of a gutter: the row, its baseline and
/// the edge.
type RowEdge = (u32, f32, f32);

/// A gutter of a region and where the rows of its two sides lie. They say on which side a
/// piece that reaches into the gutter stands (see [`Sides::place`]). The sides hold for the
/// pieces they were made from and for any part of them (a band, a stretch between rules):
/// asked about another piece, they know nothing of its row. Every place is asked of the
/// ink of the pieces (see [`Piece::ink_end`]), never of the blanks after it.
#[derive(Debug, Clone)]
struct Sides {
    /// The gutter, from its left to its right edge: edges of the steps the band was
    /// measured in (see [`best_band`]), so they move by a step with ink far from the gutter.
    /// Only whether a piece reaches into the gutter at all is asked of them, with the slack;
    /// where such a piece stands is asked of the ink of the sides.
    g0: f32,
    g1: f32,
    /// A piece reaches into the gutter, or into the ink of a row, when it reaches past its
    /// edge by more than this (see [`GUTTER_SLACK`]).
    slack: f32,
    /// The least width of a gutter of the region: [`GUTTER`] times the median text size of
    /// its pieces, the width the band of the gutter itself was held to (see [`best_band`]).
    /// A share of the text size, so a page printed larger or smaller has the same proofs.
    least: f32,
    /// Where the ink of each row of the left side ends, sorted: the right end of its pieces
    /// that end before the gutter.
    left_rows: Vec<f32>,
    /// Where the ink of each row of the right side starts, sorted: the left end of its
    /// pieces that start after the gutter.
    right_rows: Vec<f32>,
    /// Where each row with ink that starts in the ink of the left side starts, by row.
    starts: Vec<RowEdge>,
    /// Where each row with ink that ends in the ink of the right side ends, by row.
    ends: Vec<RowEdge>,
    /// The starts of `starts` and the ends of `ends`, each sorted by its place: how many
    /// rows start after a place, or end before one, is one search (see [`Sides::spans`]).
    starts_at: Vec<f32>,
    ends_at: Vec<f32>,
    /// The rows with a piece that stands over both sides (see [`Sides::over`]), sorted.
    across: Vec<u32>,
    /// A piece of the region reaches into the gutter. Without one every piece stands on
    /// its side, and the rows of the sides are not looked at.
    touched: bool,
}

impl Sides {
    /// The sides of the gutter from `g0` to `g1` in the pieces `ids`.
    fn new(p: &[Piece], ids: &[usize], (g0, g1): (f32, f32), slack: f32) -> Self {
        work::add(ids.len() as u64);
        let mut sides = Self {
            g0,
            g1,
            slack,
            least: slack * GUTTER / GUTTER_SLACK,
            left_rows: Vec::new(),
            right_rows: Vec::new(),
            starts: Vec::new(),
            ends: Vec::new(),
            starts_at: Vec::new(),
            ends_at: Vec::new(),
            across: Vec::new(),
            touched: false,
        };
        // Only a piece that reaches into the gutter asks for the rows of the sides: most
        // gutters are white from top to bottom. No part of `ids` has such a piece then
        // either, so `place` never looks at the rows that are left out here.
        if !ids.iter().any(|&i| sides.reaches(&p[i])) {
            return sides;
        }
        sides.touched = true;
        work::add(5 * ids.len() as u64);
        // The outer edge of the ink of each row among the pieces that `on` gives an edge
        // of, by row.
        let rows = |on: &dyn Fn(&Piece) -> Option<f32>, outer: fn(f32, f32) -> f32| {
            let mut ink: Vec<RowEdge> = ids
                .iter()
                .map(|&i| &p[i])
                .filter(|q| q.inked)
                .filter_map(|q| Some((q.line, q.baseline, on(q)?)))
                .collect();
            ink.sort_by_key(|r| r.0);
            let mut edges: Vec<RowEdge> = Vec::new();
            for (line, baseline, x) in ink {
                match edges.last_mut() {
                    Some(e) if e.0 == line => e.2 = outer(e.2, x),
                    _ => edges.push((line, baseline, x)),
                }
            }
            edges
        };
        let sorted = |edges: &[RowEdge]| {
            let mut v: Vec<f32> = edges.iter().map(|e| e.2).collect();
            v.sort_by(f32::total_cmp);
            v
        };
        sides.left_rows = sorted(&rows(
            &|q| (q.ink_end() <= g0 + slack).then_some(q.ink_end()),
            f32::max,
        ));
        sides.right_rows = sorted(&rows(
            &|q| (q.bounds.x0 >= g1 - slack).then_some(q.bounds.x0),
            f32::min,
        ));
        // A row has ink of a side when a piece of it starts before the ink of the left side
        // ends, or ends after the ink of the right side starts: asked of the ink of the
        // sides, not of the edges of the gutter.
        let (left, right) = sides.edges();
        sides.starts = rows(&|q| (q.bounds.x0 < left).then_some(q.bounds.x0), f32::min);
        sides.ends = rows(&|q| (q.ink_end() > right).then_some(q.ink_end()), f32::max);
        sides.starts_at = sorted(&sides.starts);
        sides.ends_at = sorted(&sides.ends);
        let mut across: Vec<u32> = ids
            .iter()
            .map(|&i| &p[i])
            .filter(|q| sides.reaches(q) && sides.over(q) == (true, true))
            .map(|q| q.line)
            .collect();
        across.sort_unstable();
        across.dedup();
        sides.across = across;
        sides
    }

    /// Into the ink of how many rows of the left and of the right side the piece `q`
    /// reaches: the rows of the left side that end after its start, and the rows of the
    /// right side that start before its end.
    fn support(&self, q: &Piece) -> (usize, usize) {
        let (left, right) = (&self.left_rows, &self.right_rows);
        (
            left.len() - left.partition_point(|&end| end <= q.bounds.x0 + self.slack),
            right.partition_point(|&start| start < q.ink_end() - self.slack),
        )
    }

    /// Whether the piece `q` stands over the left and over the right side: it reaches into
    /// the ink of more than half of the rows of that side (see [`Sides::support`]).
    fn over(&self, q: &Piece) -> (bool, bool) {
        let (left, right) = self.support(q);
        (
            2 * left > self.left_rows.len(),
            2 * right > self.right_rows.len(),
        )
    }

    /// Where the ink of the two sides ends and starts: the end of the longest row of the
    /// left side and the start of the first row of the right side, the white space that
    /// every row of the sides leaves free. The longest line of a column reaches the edge of
    /// its frame, ragged or not. For a side without rows, the edge of the gutter.
    fn edges(&self) -> (f32, f32) {
        (
            self.left_rows.last().copied().unwrap_or(self.g0),
            self.right_rows.first().copied().unwrap_or(self.g1),
        )
    }

    /// Whether the piece `q`, with its outer edge at `x`, shares that edge with the row
    /// before or after it among `edges` (the rows of one side with their outer edges): the
    /// next row of that side over or under it, at most [`BLOCK_PITCH`] sizes away (the lines
    /// of one block follow each other), has its edge within [`ROW_EDGE`] of the size of `q`
    /// (the edges of the lines of one block differ by the side bearings of their first and
    /// last letters, a share of the size). A line of a block set flush left or flush right
    /// shares the edge of the block with the line next to it; a heading centred over the
    /// columns shares no edge. Gives two answers: whether it shares the edge with a row of
    /// that side only, and whether with a row that stands across (see [`Sides::across`]).
    fn shares(&self, edges: &[RowEdge], q: &Piece, x: f32) -> (bool, bool) {
        let at = edges.partition_point(|e| e.0 < q.line);
        let after = at + usize::from(edges.get(at).is_some_and(|e| e.0 == q.line));
        let (mut side, mut across) = (false, false);
        for &(line, baseline, edge) in [
            at.checked_sub(1).and_then(|k| edges.get(k)),
            edges.get(after),
        ]
        .into_iter()
        .flatten()
        {
            if (baseline - q.baseline).abs() <= BLOCK_PITCH * q.size
                && (edge - x).abs() <= ROW_EDGE * q.size
            {
                if self.across.binary_search(&line).is_ok() {
                    across = true;
                } else {
                    side = true;
                }
            }
        }
        (side, across)
    }

    /// Where the piece `q` stands. A piece whose ink ends before the gutter stands on the
    /// left, one that starts after it on the right. A piece that reaches into the gutter
    /// has one of three places, each by support of the rows of the sides, never by how far
    /// it reaches past an edge of the gutter (those are edges of the steps the band was
    /// measured in) and never by a guess from its lengths:
    ///
    /// - **Across**, proven one of three ways. It stands over both sides: it reaches into
    ///   the ink of more than half of the rows of each (see [`Sides::over`]), a title, a
    ///   note or a line of text from the lines of the left side to those of the right one.
    ///   Or it is a drawn box (see [`Piece::boxed`]) from where the ink of the left side
    ///   ends to where the ink of the right side starts (see [`Sides::edges`]): the cell of
    ///   a header bar is drawn across the white space as a rule across it is. Or it shares
    ///   an edge with the row next to it that itself stands across (see [`Sides::shares`]):
    ///   a shorter line of a paragraph across the page.
    /// - **A side**, proven one of two ways. By the edge of the row next to it on that side
    ///   only: it starts where the row before or after it with ink on the left starts, or
    ///   ends where the one on the right ends (the longest line of a block set flush right
    ///   that starts before the end of a long line or two of the left side; the ragged end
    ///   of a line of the left side that runs past the start of a line or two of the right
    ///   side). Or by the rows of that side alone: it reaches into the ink of rows of that
    ///   side and of no row of the other, and either it stands over that side (a line of
    ///   the left column that runs through the white space up to where the right block
    ///   starts), or it leaves the width of a gutter (see [`GUTTER`]) free before the ink
    ///   of the other side: counted as a row of its side, the white space between the
    ///   sides is still a gutter (a value under its label that runs a little past the
    ///   end of the label).
    /// - **Unsure** else. A heading centred over the columns, a value that runs out of its
    ///   column into a few rows of the other side, the number of a box in the white space,
    ///   the period beside a title: the rows of the sides do not say where it stands, and
    ///   no further cue is tried (not its larger part, not the ink nearest to it, not
    ///   where its middle lies). The output changes instead: the row of the piece stays
    ///   whole (see [`rows_whole`]).
    ///
    /// The rows of the sides say nothing of the row of the piece itself. One more proof of
    /// a side holds only beside other ink of that row (see [`Sides::near`] and [`places`]).
    ///
    /// White space crosses nothing (a blank string that a print run draws under a row of
    /// fields is no text across the columns): it goes with the side its start lies on.
    fn place(&self, q: &Piece) -> Place {
        let (x0, x1) = (q.bounds.x0, q.ink_end());
        if !q.inked {
            return if x0 < (self.g0 + self.g1) / 2.0 {
                Place::Left
            } else {
                Place::Right
            };
        }
        if !self.reaches(q) {
            return if x1 <= self.g0 + self.slack {
                Place::Left
            } else {
                Place::Right
            };
        }
        let (in_left, in_right) = self.support(q);
        let (over_left, over_right) = self.over(q);
        if over_left && over_right {
            return Place::Across;
        }
        // A drawn box from the ink of the left side to the ink of the right side is drawn
        // across the white space between them.
        let (ink_left, ink_right) = self.edges();
        if q.boxed.is_some() && x0 <= ink_left + self.slack && x1 >= ink_right - self.slack {
            return Place::Across;
        }
        let left = self.shares(&self.starts, q, x0);
        let right = self.shares(&self.ends, q, x1);
        match (left.0, right.0) {
            (true, false) => Place::Left,
            (false, true) => Place::Right,
            _ if left.1 || right.1 => Place::Across,
            _ if in_left > 0 && in_right == 0 && (over_left || ink_right - x1 >= self.least) => {
                Place::Left
            }
            _ if in_right > 0 && in_left == 0 && (over_right || x0 - ink_left >= self.least) => {
                Place::Right
            }
            _ => Place::Unsure,
        }
    }

    /// The side the piece `q` stands near, for a piece whose place is unsure (see
    /// [`Sides::place`]) in the white space between the sides, in the ink of no row: it
    /// starts less than a gutter's width (see [`Sides::least`]) after the ink of the one
    /// side and leaves a gutter free before the other. Only as a piece of the near side
    /// does it leave the gutter standing (an amount set flush right in its box, past the
    /// end of the longest line of the boxes, with its currency sign at the left of its
    /// row). With a gutter free on both sides or on neither, nothing says whose it is.
    ///
    /// This is a proof only beside other ink of the row of the piece (see [`places`]):
    /// the ink of the sides is the end of the longest row of the one and the start of the
    /// first row of the other, one row each, and says nothing of a piece alone on a row
    /// of its own. A mark or a short word between two lines of a column, close to its
    /// ragged edge, is no line of that column by this (review R1 of Task 31b); the rows
    /// of the sides over and under it decide (see [`rows_whole`]).
    ///
    /// The width asked is a share of the text size, not of the page. It is a step: a
    /// piece a little under a gutter's width before the far side is unsure, and its row
    /// is kept whole, the honest read. On the public form the rule was made for, the
    /// amount ends 5.7 points before the far side, where the least gutter is 5.4 (0.8
    /// times a size of 6.7): a margin of 0.3 points, about 5 % of a size.
    fn near(&self, q: &Piece) -> Option<Place> {
        if !self.reaches(q) || self.support(q) != (0, 0) {
            return None;
        }
        let (x0, x1) = (q.bounds.x0, q.ink_end());
        let (ink_left, ink_right) = self.edges();
        let (after, before) = (x0 - ink_left, ink_right - x1);
        if before >= self.least && after < self.least {
            Some(Place::Left)
        } else if after >= self.least && before < self.least {
            Some(Place::Right)
        } else {
            None
        }
    }

    /// Whether the piece `q` runs the whole width of the left, of the right side: it starts
    /// where most of the other rows with ink in the left side start, or before them (the
    /// rows of `starts` whose start is not left of its own, of those that are not its own
    /// row); it ends where most of the other rows with ink in the right side end, or after
    /// them. A piece from the outer edge of a side to the gutter is no line of the other
    /// side, whatever else stands on its row: a line of a column does not start at the far
    /// edge of the column beside it. A side without another row says nothing.
    ///
    /// The rows are counted by a search in their sorted edges, the row of the piece itself
    /// taken out by one more: the work of a piece does not grow with the rows of the
    /// region (a form whose rows each hold such a piece is read in near linear work).
    fn spans(&self, q: &Piece) -> (bool, bool) {
        work::add(1);
        // The edge of the row of `q` among `edges` (sorted by row), if it has one.
        let own = |edges: &[RowEdge]| {
            let at = edges.partition_point(|e| e.0 < q.line);
            edges.get(at).filter(|e| e.0 == q.line).map(|e| e.2)
        };
        let from = q.bounds.x0 - self.slack;
        let to = q.ink_end() + self.slack;
        let most = |all: usize, held: usize, own: Option<bool>| {
            let (all, held) = match own {
                Some(holds) => (all - 1, held - usize::from(holds)),
                None => (all, held),
            };
            2 * held > all
        };
        let starts = &self.starts_at;
        let ends = &self.ends_at;
        (
            most(
                starts.len(),
                starts.len() - starts.partition_point(|&start| start < from),
                own(&self.starts).map(|start| start >= from),
            ),
            most(
                ends.len(),
                ends.partition_point(|&end| end <= to),
                own(&self.ends).map(|end| end <= to),
            ),
        )
    }

    /// Whether the piece `q` has ink that reaches into the gutter.
    fn reaches(&self, q: &Piece) -> bool {
        q.inked && q.ink_end() > self.g0 + self.slack && q.bounds.x0 < self.g1 - self.slack
    }
}

/// Where a piece goes when its region is parted at a gutter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Goes {
    /// With the left side.
    Left,
    /// With the right side.
    Right,
    /// Alone, between the bands of the sides: a piece across.
    Across,
    /// With its whole row, as laid out, between the bands of the sides.
    Row,
    /// After both sides of its band: an unsure piece that is the only ink of its row, where
    /// both sides run on through it.
    After,
    /// Alone, between the parts of the left side over and under it: an unsure piece that is
    /// the only ink of its row, where the left side alone breaks its pitch. The right side
    /// is not parted.
    HeadLeft,
    /// The same for the right side: between its parts, the left side not parted.
    HeadRight,
}

/// Where each of the pieces `ids`, sorted by row, stands at the gutter of `sides`: its
/// place by the rows of the sides (see [`Sides::place`]), and for a piece that those leave
/// unsure the side it stands near (see [`Sides::near`]), where its row has other ink with
/// a proven side. A piece that is the only ink of its row, or stands beside unsure pieces
/// only, stays unsure: nothing of its row says that it is a piece of a line. The work is
/// counted by the callers.
fn places(p: &[Piece], ids: &[usize], sides: &Sides) -> Vec<Place> {
    let mut all: Vec<Place> = ids.iter().map(|&i| sides.place(&p[i])).collect();
    if !sides.touched {
        return all;
    }
    let mut a = 0;
    while a < ids.len() {
        let line = p[ids[a]].line;
        let z = a + ids[a..].iter().take_while(|&&i| p[i].line == line).count();
        let beside =
            (a..z).any(|k| p[ids[k]].inked && matches!(all[k], Place::Left | Place::Right));
        if beside {
            for k in a..z {
                if all[k] == Place::Unsure
                    && let Some(side) = sides.near(&p[ids[k]])
                {
                    all[k] = side;
                }
            }
        }
        a = z;
    }
    all
}

/// What a side says at the height of a piece that is the only ink of its row (see
/// [`flow_at`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    /// The side runs on through the height at its line pitch.
    Runs,
    /// The side has a row over and a row under the height, more than its pitch apart.
    Breaks,
    /// The side has no pitch, or no row over or under the height: it says nothing.
    Unknown,
}

/// What [`rows_whole`] gives: where each piece goes, how many pieces stand between the bands,
/// the pieces that start a line, and where each piece stands.
type Whole = (Vec<Goes>, usize, Vec<usize>, Vec<Place>);

/// Where each of the pieces `ids`, sorted by row, goes at the gutter of `sides`; how many
/// pieces stand between the bands of the sides (across, or unsure in a row kept whole);
/// and the pieces that start a line of their own on their side. A piece with a proven
/// place goes there. A piece whose place is unsure (see [`places`]) is not given a side
/// by a further cue; its row stays whole, as laid out:
///
/// - The other ink of its row stands on one side only: the piece goes there with its row
///   (a value that runs out of its column stays with its label, a mark in the white space
///   with the line it stands on), and the sides read as if it reached into nothing. When
///   the piece reaches into the ink of a row of the other side (see [`Sides::support`]),
///   it is a line of its own there: it reads at its place in its row, and is not said to
///   be one line with the text beside it (a title that runs from one block into the next, with
///   a number far right on its row).
///   A piece that runs the whole width of the other side (see [`Sides::spans`]: a row of
///   labels set as one piece from the left edge of the left block into the right one,
///   with a value of the right block on its row) is no line of the side its row's ink
///   stands on: its row is kept whole as in the last case, and it starts a line.
/// - It is the only ink of its row (a mark, a page number, a short word or a heading on a
///   baseline of its own in the white space): there is no row to keep whole, and the
///   rows of the sides say whether the columns end at its height (see [`flow_at`]). Where
///   both sides run on through it at their own line pitch (a mark between two lines of
///   prose), it parts nothing and counts against nothing: the band is read on its two
///   sides as if it were not there, and the piece after them, never between them (where
///   it reads against two blocks that do not depend on it is free). Where one side breaks
///   its pitch at the piece and the other runs on (a heading over a paragraph of one
///   column, beside a sentence of the other that goes on), the piece goes with the side
///   that breaks: it reads alone between the parts of that side over and under it, before
///   what it heads, and the running side stays one flow, never parted (design ruling of
///   fix round 8 of Task 31b; if the piece is no heading of that side, it reads one side
///   early or late against a block that does not depend on it, which is free). Where both
///   sides break their pitch at the piece (a heading between two bands of columns, a
///   label over the two halves of a form), or a side says nothing (it ends there, or has
///   no pitch), which side breaks is not shown: the piece is a row of its own between
///   the bands, read before what follows it.
/// - Else (ink of its row on both sides) no side takes the row
///   whole: the row is read at its height, between the bands over it and under it, every
///   piece of it from left to right. A title keeps the period on its row, a row of labels
///   stays over the row of its values, a heading alone on its row reads before the columns
///   under it. The parts of such a row (the pieces of the left side, the ones between,
///   those of the right side) follow each other on their row, each a group of its own:
///   the gutter stands, so they are not said to be one block.
///
/// A wrong side parts a label from its value or puts a block into a sentence; a row kept
/// whole costs a break of the columns at that row, and reads no value wrong.
///
/// The last part of the answer is where each piece stands (see [`places`]).
fn rows_whole(p: &[Piece], ids: &[usize], sides: &Sides) -> Whole {
    work::add(ids.len() as u64);
    let mut goes: Vec<Goes> = Vec::with_capacity(ids.len());
    // A gutter that no piece reaches into has every piece on its side.
    if !sides.touched {
        let all: Vec<Place> = ids.iter().map(|&i| sides.place(&p[i])).collect();
        goes.extend(all.iter().map(|&s| match s {
            Place::Left => Goes::Left,
            _ => Goes::Right,
        }));
        return (goes, 0, Vec::new(), all);
    }
    work::add(3 * ids.len() as u64);
    let all = places(p, ids, sides);
    // The baselines of the rows of each side, ascending: the rows with ink of a proven
    // place there.
    let near = BASELINE_NEAR * sides.least / GUTTER;
    let rows_of = |side: Place| {
        let mut rows: Vec<(u32, f32)> = Vec::new();
        let mut sizes: Vec<f32> = Vec::new();
        for (&i, _) in ids
            .iter()
            .zip(&all)
            .filter(|&(&i, &s)| p[i].inked && s == side)
        {
            sizes.push(p[i].size);
            match rows.last_mut() {
                Some(r) if r.0 == p[i].line => r.1 = r.1.max(p[i].baseline),
                _ => rows.push((p[i].line, p[i].baseline)),
            }
        }
        let mut ys: Vec<f32> = rows.iter().map(|r| r.1).collect();
        ys.sort_by(f32::total_cmp);
        // Two baselines within `BASELINE_NEAR` of the text size are one (a raised mark, a
        // word set a hair lower): the step between them is no step of the lines.
        ys.dedup_by(|later, kept| *later - *kept <= near);
        // The pitch of lines of one column is at least `LEAST_PITCH` of their text size.
        sizes.sort_by(f32::total_cmp);
        let size = sizes.get(sizes.len() / 2).copied().unwrap_or(0.0);
        let pitch = pitch_of(&ys).filter(|&pitch| pitch >= LEAST_PITCH * size);
        (ys, pitch)
    };
    let (left_rows, right_rows) = (rows_of(Place::Left), rows_of(Place::Right));
    let mut breaks = Vec::new();
    let mut between = 0;
    let mut a = 0;
    while a < ids.len() {
        let line = p[ids[a]].line;
        let z = a + ids[a..].iter().take_while(|&&i| p[i].line == line).count();
        let places = &all[a..z];
        let unsure = places.iter().filter(|&&s| s == Place::Unsure).count();
        // The ink of the row with a proven side.
        let on = |side: Place| {
            ids[a..z]
                .iter()
                .zip(places)
                .any(|(&i, &s)| p[i].inked && s == side)
        };
        // What a side says at the unsure pieces of a row without ink of a side: the same
        // at each of them, else nothing.
        let flow = |rows: &(Vec<f32>, Option<f32>)| {
            let mut at = ids[a..z]
                .iter()
                .zip(places)
                .filter(|&(_, &s)| s == Place::Unsure)
                .map(|(&i, _)| flow_at(&rows.0, rows.1, p[i].baseline));
            let first = at.next().unwrap_or(Flow::Unknown);
            if at.all(|f| f == first) {
                first
            } else {
                Flow::Unknown
            }
        };
        let across = places.contains(&Place::Across);
        // An unsure piece of the row runs the whole width of the left, of the right side
        // (see [`Sides::spans`]): asked only where the row would go to the other side.
        let spans = |left: bool| {
            ids[a..z]
                .iter()
                .zip(places)
                .filter(|&(_, &s)| s == Place::Unsure)
                .any(|(&i, _)| {
                    let (l, r) = sides.spans(&p[i]);
                    if left { l } else { r }
                })
        };
        let mut spanned = false;
        let whole = match (unsure > 0, on(Place::Left), on(Place::Right)) {
            (false, _, _) => None,
            // The row has ink of one side only, but its unsure piece runs the whole
            // width of the other side: it has ink of both, and no side takes the row.
            (true, true, false) if spans(false) => {
                spanned = true;
                Some(Goes::Row)
            }
            (true, false, true) if spans(true) => {
                spanned = true;
                Some(Goes::Row)
            }
            (true, true, false) => Some(Goes::Left),
            (true, false, true) => Some(Goes::Right),
            (true, true, true) => Some(Goes::Row),
            // No ink of a side beside it: there is no row to keep whole.
            (true, false, false) if across => Some(Goes::Row),
            (true, false, false) => Some(match (flow(&left_rows), flow(&right_rows)) {
                // The columns run on through it: it parts nothing.
                (Flow::Runs, Flow::Runs) => Goes::After,
                // One side breaks its pitch at it and the other runs on: it heads what
                // follows on the side that breaks, and the running side is not parted.
                (Flow::Breaks, Flow::Runs) => Goes::HeadLeft,
                (Flow::Runs, Flow::Breaks) => Goes::HeadRight,
                // Both sides break their pitch at it, or a side says nothing: which side
                // breaks is not shown, and it is a row of its own between the bands.
                _ => Goes::Row,
            }),
        };
        // An unsure piece that goes to a side with ink in a row of the other side starts
        // a line.
        if let Some(side @ (Goes::Left | Goes::Right)) = whole {
            breaks.extend(
                ids[a..z]
                    .iter()
                    .zip(places)
                    .filter(|&(&i, &s)| {
                        let (in_left, in_right) = sides.support(&p[i]);
                        let other = if side == Goes::Left {
                            in_right
                        } else {
                            in_left
                        };
                        s == Place::Unsure && other > 0
                    })
                    .map(|(&i, _)| i),
            );
        }
        // A piece over the whole width of one side on a row with the ink of the other is
        // not said to be one line with that ink: it starts a line.
        if spanned {
            breaks.extend(
                ids[a..z]
                    .iter()
                    .zip(places)
                    .filter(|&(_, &s)| s == Place::Unsure)
                    .map(|(&i, _)| i),
            );
        }
        between += places.iter().filter(|&&s| s == Place::Across).count();
        if whole == Some(Goes::Row) {
            between += unsure;
            goes.extend(std::iter::repeat_n(Goes::Row, z - a));
        } else {
            goes.extend(places.iter().map(|&s| match s {
                Place::Left => Goes::Left,
                Place::Right => Goes::Right,
                Place::Across => Goes::Across,
                Place::Unsure => whole.unwrap_or(Goes::After),
            }));
        }
        a = z;
    }
    (goes, between, breaks, all)
}

/// The line pitch of a side whose rows have the baselines `ys`, ascending: the median
/// step between them, the lower one of two (of a column of two paragraphs with one break
/// the pitch is the step of its lines, not the break). A side of fewer than three rows
/// has none: one step alone is no pitch that a second one could keep or break. The
/// caller gives each baseline once: rows whose baselines are one (see [`BASELINE_NEAR`])
/// would make a step of a hair the pitch, and every line a break of it.
fn pitch_of(ys: &[f32]) -> Option<f32> {
    let mut steps: Vec<f32> = ys.windows(2).map(|w| w[1] - w[0]).collect();
    if steps.len() < 2 {
        return None;
    }
    steps.sort_by(f32::total_cmp);
    Some(steps[(steps.len() - 1) / 2])
}

/// The lines of one column stand at least this many text sizes apart, baseline to
/// baseline: closer, the letters of one line would touch those of the next (text set
/// solid stands one size apart, tight headings a little less). A side whose rows stand
/// closer than that on median is no column of lines: it holds several columns side by
/// side whose baselines do not meet, and the steps between their rows are no line pitch
/// that a piece beside them could break (traced on a public page, arxiv-2408.09869 page
/// 7: the small copy of a page of two columns, as the right side of a gutter, had a
/// "pitch" of a third of its text size, and every line of an author block beside it was
/// taken for a break of that side). Such a side says nothing (see [`flow_at`]). A share
/// of the text size: it holds at every scale of the page.
const LEAST_PITCH: f32 = 0.8;

/// What a side whose rows have the baselines `ys`, ascending, at the line pitch `pitch`
/// (see [`pitch_of`]) says at the height `y`. It runs on through `y` when it has a row
/// over and a row under `y`, and the two are one pitch of that side apart, within
/// [`LONE_PITCH`] of it (the measure a block of one pitch is held to beside a lone row).
/// A column that goes on through the height of a piece in the white space beside it does
/// not end there: the piece is no heading of what follows on that side. A wider step
/// between those two rows is a break of the column (the white space over a heading, the
/// end of a paragraph). A side without a row over or under `y`, or without a pitch, says
/// nothing: neither that it runs on nor that it breaks.
fn flow_at(ys: &[f32], pitch: Option<f32>, y: f32) -> Flow {
    let k = ys.partition_point(|&b| b < y);
    let (Some(pitch), Some(under), Some(over)) =
        (pitch, k.checked_sub(1).and_then(|k| ys.get(k)), ys.get(k))
    else {
        return Flow::Unknown;
    };
    if over - under <= (1.0 + LONE_PITCH) * pitch {
        Flow::Runs
    } else {
        Flow::Breaks
    }
}

/// The stretches of the sorted pieces `ids` that stand between two rules, one over the
/// other: each as the range of its indices. The upper rule is one of `upper` (the heights of
/// the rules drawn in pieces that reach across the gutter), the lower one of `closing` (the
/// heights of all rules), both sorted. A stretch starts at the first piece under its upper
/// rule and ends before the first piece under the next rule of any kind; white space goes
/// with the text before it. Only a stretch with positive evidence that it is rows of one
/// table is one:
/// - [`RULED_ROWS`] rows or more of one geometry hold text on both sides of the gutter (from
///   `g0` to `g1`, pieces across it aside): rows that the gutter would cut apart, each with as
///   many pieces at the same edges (see [`ROW_EDGE`]);
/// - each joint of the upper rule over the stretch (`joints`: the height and place of each
///   joint, sorted) lies in white space of all its rows: the pieces of the rule are the
///   columns of the rows;
/// - the row right over the upper rule is the header of its rows (see [`header_row`]): titles
///   over their columns, as many as the rows have pieces, or one less (a title over two
///   columns). It starts the stretch.
///
/// A side heading of two lines beside the lines of a form has rows on both sides of the gutter
/// too, but no titles over a rule in pieces. Lines of a label and its amount between two
/// rules in pieces (totals under a table) have a line with an amount over their upper rule,
/// no titles. The stretch that is all of `ids` is none: it is looked at alone already.
fn between_rules(
    p: &[Piece],
    ids: &[usize],
    (upper, closing): (&[f32], &[f32]),
    joints: &[(f32, f32)],
    sides: &Sides,
) -> Vec<(usize, usize)> {
    let slack = sides.slack;
    if upper.is_empty() || joints.is_empty() {
        return Vec::new();
    }
    // How many pieces the rows of one geometry with text on both sides of the gutter have,
    // when the stretch holds enough of them: rows of as many pieces, each piece at the left
    // or the right edge of the piece of its place in the first such row.
    let rows = |a: usize, z: usize| -> Option<usize> {
        work::add((z - a) as u64);
        // The first row of each number of pieces, and how many rows are like it.
        let mut like: Vec<(Vec<usize>, usize)> = Vec::new();
        let mut k = a;
        while k < z {
            let line = p[ids[k]].line;
            let (mut left, mut right) = (false, false);
            let mut row: Vec<usize> = Vec::new();
            while k < z && p[ids[k]].line == line {
                let q = &p[ids[k]];
                if q.inked {
                    row.push(ids[k]);
                    match sides.place(q) {
                        Place::Left => left = true,
                        Place::Right => right = true,
                        Place::Across | Place::Unsure => {}
                    }
                }
                k += 1;
            }
            if !(left && right) {
                continue;
            }
            work::add(like.len() as u64);
            match like.iter_mut().find(|l| l.0.len() == row.len()) {
                Some((first, n)) => {
                    let same = first.iter().zip(&row).all(|(&f, &r)| {
                        let (f, r) = (&p[f], &p[r]);
                        let near = ROW_EDGE * f.size.max(r.size);
                        (f.bounds.x0 - r.bounds.x0).abs() <= near
                            || (f.bounds.x1 - r.bounds.x1).abs() <= near
                    });
                    *n += usize::from(same);
                }
                None => like.push((row, 1)),
            }
        }
        like.iter().find(|l| l.1 >= RULED_ROWS).map(|l| l.0.len())
    };
    // Whether the joints of the rule at `y` mark columns of the stretch: one or more of
    // them stand over it, each in white space of all its rows. The ink of the stretch is
    // joined to spans once (each piece without the slack at its two ends), and each joint
    // is looked up in them: the work grows with the pieces and the joints, not with their
    // product.
    let marked = |a: usize, z: usize, y: f32| {
        let mut ink: Vec<(f32, f32)> = ids[a..z]
            .iter()
            .map(|&i| &p[i])
            .filter(|q| q.inked)
            .map(|q| (q.bounds.x0, q.bounds.x1))
            .collect();
        let lo = ink.iter().map(|s| s.0).fold(f32::MAX, f32::min);
        let hi = ink.iter().map(|s| s.1).fold(f32::MIN, f32::max);
        ink.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut spans: Vec<(f32, f32)> = Vec::new();
        for (x0, x1) in ink {
            let (x0, x1) = (x0 + slack, x1 - slack);
            match spans.last_mut() {
                Some(s) if x0 < s.1 => s.1 = s.1.max(x1),
                _ if x0 < x1 => spans.push((x0, x1)),
                _ => {}
            }
        }
        let from = joints.partition_point(|j| j.0 < y - SNAP);
        let of_rule = &joints[from..];
        let of_rule = &of_rule[..of_rule.partition_point(|j| j.0 <= y + SNAP)];
        work::add((2 * (z - a) + of_rule.len()) as u64);
        let mut over = of_rule.iter().map(|j| j.1).filter(|&x| x > lo && x < hi);
        // A joint lies in ink when it lies inside a span: after its start and before its end.
        let in_ink = |x: f32| {
            let k = spans.partition_point(|s| s.0 < x);
            k > 0 && x < spans[k - 1].1
        };
        let mut any = false;
        over.all(|x| {
            any = true;
            !in_ink(x)
        }) && any
    };
    // The lowest of the sorted heights `rules` between `top` and `low`.
    let under = |rules: &[f32], top: f32, low: f32| {
        rules
            .get(rules.partition_point(|&y| y <= top))
            .copied()
            .filter(|&y| y < low)
    };
    work::add(ids.len() as u64);
    // Where the text goes on under a rule that the text before it stands over, with the
    // height of that rule when it is an upper rule: at the first piece with ink whose top
    // lies under the rule.
    let mut starts: Vec<(usize, Option<f32>)> = Vec::new();
    let mut low = f32::MAX;
    for (k, &i) in ids.iter().enumerate() {
        if !p[i].inked {
            continue;
        }
        let top = p[i].bounds.y1;
        if under(closing, top, low).is_some() {
            starts.push((k, under(upper, top, low)));
            low = f32::MAX;
        }
        // The text stands over a rule that lies under its baseline, or on it (see
        // [`BASELINE_NEAR`]): the descenders of a row may reach under the rule that
        // underlines it.
        low = low.min(p[i].baseline + BASELINE_NEAR * p[i].size);
    }
    starts
        .windows(2)
        .filter_map(|w| {
            let ((a, y), (z, _)) = (w[0], w[1]);
            let y = y?;
            let cells = rows(a, z)?;
            if !marked(a, z, y) {
                return None;
            }
            let (head, titles) = header_row(p, ids, (a, z), y)?;
            (titles == cells || titles + 1 == cells).then_some((head, z))
        })
        .filter(|&(a, z)| z - a < ids.len())
        .collect()
}

/// The text between two rules is a block of rows when it holds at least this many rows of
/// one geometry with text on both sides of a gutter.
const RULED_ROWS: usize = 2;

/// Two pieces stand at one edge of a column when their left or their right ends are within
/// this share of their text size.
const ROW_EDGE: f32 = 0.25;

/// The baseline of a header stands at most this many of its sizes over the rule under it. A
/// label that sits on its rule (a title on an underline, a field of a form on its line) has
/// its baseline on the rule: up to [`BASELINE_NEAR`] of its size under the rule's middle is
/// on it still, as two baselines that near are one.
const HEADER_OVER_RULE: f32 = 1.0;

/// Whole numbers in a row over a rule are titles when the row holds at least this many of
/// them: one number says nothing (it is the amount of the label beside it as well as a
/// year), two or more side by side are a run of years or of column numbers.
///
/// Known gap (review Q6 of Task 31b, owner Task 26, which reads the amounts of labels and
/// values): two whole-number amounts side by side (`Total 12 340`: two counts, or amounts
/// in a currency without decimals) are titles by this count too, and their row is taken
/// for the header of the rows under the rule. A whole number alone does not say whether
/// it is an amount or a year. The gap is pinned in
/// `two_whole_amounts_over_the_rule_are_titles_still`.
const TITLE_NUMBERS: usize = 2;

/// Where the header row of the stretch `a..z` of the sorted pieces `ids` starts, under a rule
/// at the height `rule`, and how many titles it has: the index of the first piece of the row
/// right over the rule, when that row is tied to the rows under the rule. Its baselines stand
/// on the rule or over it and close to it (see [`HEADER_OVER_RULE`]), it has two pieces with ink
/// or more, none of them reads as an amount or a date (a title is words, or one of
/// [`TITLE_NUMBERS`] whole numbers or more in the row, see [`reads_as_number`]: a line of a
/// label and its amount over the rule is a row of totals, no header, also when the amount
/// is a whole number), and each of them stands over
/// text of the stretch (their widths overlap) but over no two pieces of one row of it: titles
/// over their columns. A line of prose over the rule is one piece, and a title across two
/// columns is not taken: such a row stays with the text over the rule.
fn header_row(
    p: &[Piece],
    ids: &[usize],
    (a, z): (usize, usize),
    rule: f32,
) -> Option<(usize, usize)> {
    let last = ids[..a].iter().rposition(|&i| p[i].inked)?;
    let line = p[ids[last]].line;
    let first = ids[..=last]
        .iter()
        .rposition(|&i| p[i].line != line)
        .map_or(0, |k| k + 1);
    let row: Vec<usize> = ids[first..=last]
        .iter()
        .copied()
        .filter(|&i| p[i].inked)
        .collect();
    work::add((row.len() * (z - a)) as u64);
    // A whole number is a title only among others like it: years over their columns, the
    // numbers of the columns. One whole number beside a label is the amount of that label
    // (`Total 250`: a count, or an amount in a currency without decimals).
    let numbers = row.iter().filter(|&&i| p[i].number).count();
    let amount = |q: &Piece| q.value && !(q.number && numbers >= TITLE_NUMBERS);
    if row.len() < 2 || row.iter().any(|&i| amount(&p[i])) {
        return None;
    }
    let close = row.iter().all(|&i| {
        let over = p[i].baseline - rule;
        (-BASELINE_NEAR * p[i].size..=HEADER_OVER_RULE * p[i].size).contains(&over)
    });
    if !close {
        return None;
    }
    let over = |h: usize, i: usize| {
        p[i].inked && p[h].bounds.x0 < p[i].bounds.x1 && p[i].bounds.x0 < p[h].bounds.x1
    };
    let tied = row.iter().all(|&h| {
        let mut rows: Vec<u32> = ids[a..z]
            .iter()
            .filter(|&&i| over(h, i))
            .map(|&i| p[i].line)
            .collect();
        let n = rows.len();
        rows.dedup();
        n > 0 && rows.len() == n
    });
    tied.then_some((first, row.len()))
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
fn gutter(p: &[Piece], ids: &[usize]) -> Option<(Sides, Why)> {
    let inked: Vec<usize> = ids.iter().copied().filter(|&i| p[i].inked).collect();
    if inked.is_empty() {
        return None;
    }
    let allowed = (CROSSERS * inked.len() as f32).floor() as i32;
    let mut sizes: Vec<f32> = inked.iter().map(|&i| p[i].size).collect();
    sizes.sort_by(f32::total_cmp);
    let size = sizes[sizes.len() / 2];
    let (g0, g1) = best_band(p, &inked, allowed, size)?;
    let sides = Sides::new(p, ids, (g0, g1), GUTTER_SLACK * size);
    let why = split_at(p, ids, &sides, allowed, size);
    if debug_cut() && why.is_none() {
        eprintln!("CUT   band {g0:.1}..{g1:.1} is no gutter (allowed {allowed})");
        for q in ids
            .iter()
            .map(|&i| &p[i])
            .filter(|q| sides.reaches(q) || q.boxed.is_some())
        {
            eprintln!(
                "CUT   reaches in: line {} x {:.1}..{:.1} boxed {:?} stands {:?}",
                q.line,
                q.bounds.x0,
                q.bounds.x1,
                q.boxed,
                sides.place(q)
            );
        }
    }
    Some((sides, why?))
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
/// `allowed` of them stand between the bands of its sides (see [`rows_whole`]), the blocks
/// on its sides stand side by side and are prose, lines
/// whose baselines do not line up, a lone row beside a block, columns in the shape of
/// addresses or a drawn box. `size` is the median size of the pieces.
fn split_at(p: &[Piece], ids: &[usize], sides: &Sides, allowed: i32, size: f32) -> Option<Why> {
    // The two sides and the tests of the sides pass the pieces a few times.
    work::add(3 * ids.len() as u64);
    // The pieces between the bands of the sides (across, or unsure in a row kept whole)
    // count against the gutter: each parts the columns at its row.
    let mut sorted = ids.to_vec();
    if sides.touched {
        sorted.sort_by_key(|&i| p[i].line);
    }
    let (goes, between, _, stands) = rows_whole(p, &sorted, sides);
    if between as i32 > allowed {
        return None;
    }
    // What the sides are made of is asked of the pieces with a proven side, also those
    // of a row kept whole: a row with ink on both sides on one baseline is a row of both
    // blocks, and says that their lines meet. A piece that goes to a side with its row
    // counts there. A piece alone on its row with no proven side counts on neither.
    let (mut left, mut right) = (Vec::new(), Vec::new());
    for ((&i, go), place) in sorted.iter().zip(goes).zip(stands) {
        match (go, place) {
            (Goes::Left, _) | (Goes::Row, Place::Left) => left.push(i),
            (Goes::Right, _) | (Goes::Row, Place::Right) => right.push(i),
            _ => {}
        }
    }
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
///
/// Where the count stands at its limit (one line less that meets, on each side, and the
/// blocks would be apart), a line of a paragraph meets a line of the other side only with
/// the support of its paragraph (see [`unsupported`]): beside dense rows a line of a note
/// passes a row now and then by chance, and counted one by one such lines put the count at
/// its limit (8 of 16 lines against 7 of 15 on two pages of one public form). Lines that
/// meet by chance are no evidence of rows, and reading the blocks row by row would set the
/// lines of the note between the rows of the fields. This holds for a side that is such
/// paragraphs for the most part (the note column); one wrapped label among the rows of a
/// side leaves the count as it is.
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
    let apart = |met_left: &[bool], met_right: &[bool], spare: usize| {
        2 * count(met_right) < right.len() + 2 * spare
            && 2 * count(met_left) <= left.len() + 2 * spare
    };
    if apart(&met_left, &met_right, 0) {
        return true;
    }
    // The count stands at its limit when one line less that meets on a side would turn it:
    // weak evidence either way. There the meets are asked for the support of their
    // paragraphs; farther from the limit the count stands as it is.
    //
    // Why the limit is asked first (kept in fix round 8 of Task 31b, measured without it):
    // the paragraph rule then also turns counts that are far from the limit, and the cut
    // of two pages of a public form moves (irs-f6744 untagged pages 11 and 200: regions
    // are found in another order; the read is the same). A rule that was made for chance
    // meets at the limit does not get to move counts that no page asked it about.
    if !apart(&met_left, &met_right, 1) {
        return false;
    }
    let heads = |side: &[Vec<Part>]| -> Vec<f32> { side.iter().map(|l| head(l).0).collect() };
    // Only a side that is such paragraphs for the most part is a block of its own by them:
    // one wrapped label among the rows of a side says nothing of the side.
    let paragraphs = |side: &[Vec<Part>], met: &mut Vec<bool>| {
        let mut without = met.clone();
        if 2 * unsupported(&heads(side), &mut without) > side.len() {
            *met = without;
        }
    };
    paragraphs(left, &mut met_left);
    paragraphs(right, &mut met_right);
    apart(&met_left, &met_right, 0)
}

/// Takes the meets of the lines of a paragraph that its paragraph does not support. `ys`
/// are the baselines of the lines of one side, `met` whether each meets a line of the other
/// side. A paragraph is a run of [`LONE_BLOCK`] lines or more that follow each other at one
/// pitch (each step within [`LONE_PITCH`] of the first step of the run, the measures a block
/// of one pitch is held to beside a lone row). It is judged alone by the rule of
/// [`offset_baselines`] for a side: where fewer than half of its lines meet a line of the
/// other side, it runs at a pitch of its own past the rows beside it, and the lines that
/// pass a row do so by chance: none of its lines counts as met. Cells of rows at one pitch
/// all meet; a column of amounts with an amount on every row and on every line between
/// two rows meets with half of its lines, and stays a column of those rows. A line
/// outside such a run (a title, the two lines of a wrapped cell) is counted for itself.
/// Gives the number of lines of such paragraphs.
fn unsupported(ys: &[f32], met: &mut [bool]) -> usize {
    work::add(ys.len() as u64);
    let mut order: Vec<usize> = (0..ys.len()).collect();
    order.sort_by(|&a, &b| ys[b].total_cmp(&ys[a]));
    let step = |k: usize| ys[order[k]] - ys[order[k + 1]];
    // Each run is judged by the meets as they were found: two runs share the line between
    // them, and a meet that the run over it gives up still counts for the run under it. So
    // which run is judged first does not matter.
    let found = met.to_vec();
    let mut lines = 0;
    let mut last = None;
    let mut from = 0;
    while from + 1 < order.len() {
        let first = step(from);
        let mut to = from + 1;
        while first > 0.0 && to + 1 < order.len() && (step(to) - first).abs() <= LONE_PITCH * first
        {
            to += 1;
        }
        let run = &order[from..=to];
        if run.len() >= LONE_BLOCK && 2 * run.iter().filter(|&&l| found[l]).count() < run.len() {
            for &l in run {
                met[l] = false;
            }
            // The last line of a run may start the next one: counted once.
            lines += run.len() - usize::from(last == Some(from));
            last = Some(to);
        }
        from = to;
    }
    lines
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
            number: false,
            boxed: None,
            rows: 1,
            trail: 0.0,
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

    #[test]
    fn a_line_of_a_paragraph_meets_a_row_only_with_its_paragraph() {
        // The path of a public form (irs-f6744 untagged page 108). Rows of fields at the
        // left, at steps of their own; at the right a column of four title lines, a note
        // of eleven lines at a pitch of 7.7 and a foot line. Eight of the sixteen lines
        // of the column lie within a point of a row of the fields: the four titles and
        // the foot line but one, and four lines of the note, whose other seven lines meet
        // none. Counted line by line the column is not apart (8 of 16; without the foot
        // line 7 of 15, apart). The note runs at its own pitch past the rows: its four
        // meets are chance, and the column is a block of its own.
        let at =
            |ys: &[f32]| -> Vec<Vec<Part>> { ys.iter().map(|&y| vec![(y, 6.7, true)]).collect() };
        let fields = [
            348.0, 341.3, 336.0, 334.5, 324.9, 323.6, 316.1, 313.0, 301.9, 289.9, 277.8, 267.3,
            255.7, 247.7, 244.2, 232.2, 230.8, 221.1, 213.9, 209.2, 206.7, 205.0, 199.5, 198.1,
            192.3, 186.1, 175.0, 165.0, 161.6, 149.5,
        ];
        let column = |note: &dyn Fn(usize) -> f32| -> Vec<f32> {
            let mut ys = vec![323.9, 312.4, 277.2, 265.4];
            ys.extend((0..11).map(note));
            ys.push(149.5);
            ys
        };
        let note = column(&|n| 248.5 - 7.7 * n as f32);
        let mut met: Vec<bool> = note
            .iter()
            .map(|y| fields.iter().any(|f| (f - y).abs() <= BASELINE_NEAR * 6.7))
            .collect();
        assert_eq!(met.iter().filter(|&&m| m).count(), 8);
        assert_eq!(unsupported(&note, &mut met), 11);
        assert_eq!(met.iter().filter(|&&m| m).count(), 4);
        assert!(offset_baselines(&at(&fields), &at(&note), 6.7));
        // The other way, the share of the side moved: with seven more lines over the
        // column at steps of their own, four of them on rows of the fields, the count is
        // at its limit again (12 of 23), but the note is 11 of the 23 lines of the column,
        // not most of it: one paragraph among other lines says nothing of the column, and
        // the count stands. Without two of the seven (one on a row, one not) the note is
        // 11 of 21 lines, most of the column, and the column is a block of its own.
        let mut titled = note.clone();
        titled.extend([450.0, 425.0, 410.0, 400.0, 385.0, 360.0, 470.0]);
        let mut wide = fields.to_vec();
        wide.extend([470.0, 450.0, 425.0, 410.0]);
        assert!(!offset_baselines(&at(&wide), &at(&titled), 6.7));
        assert!(offset_baselines(
            &at(&wide),
            &at(&titled[..titled.len() - 2]),
            6.7
        ));
        // The other way, the support count moved: with two more rows of fields, at the
        // baselines of the second and the fourth line of the note, six of its eleven
        // lines meet a row. Most of the paragraph stands on rows, its meets count, and the
        // column is no block of its own.
        let mut more = fields.to_vec();
        more.extend([240.8, 225.4]);
        let mut met: Vec<bool> = note
            .iter()
            .map(|y| more.iter().any(|f| (f - y).abs() <= BASELINE_NEAR * 6.7))
            .collect();
        let before = met.clone();
        assert_eq!(before.iter().filter(|&&m| m).count(), 10);
        unsupported(&note, &mut met);
        assert_eq!(met, before);
        assert!(!offset_baselines(&at(&more), &at(&note), 6.7));
        // Farther from the limit the count stands: with one more row of fields, on the
        // baseline of the fourth title line, 9 of the 16 lines meet a row. One line less
        // would not turn the count (8 of 16 is not fewer than half), so the support of
        // the paragraph is not asked, though seven of its eleven lines still meet none.
        let mut far = fields.to_vec();
        far.push(265.4);
        let met = |ys: &[f32]| {
            note.iter()
                .filter(|&&y| ys.iter().any(|f| (f - y).abs() <= BASELINE_NEAR * 6.7))
                .count()
        };
        assert_eq!((met(&fields), met(&far)), (8, 9));
        assert!(!offset_baselines(&at(&far), &at(&note), 6.7));
        // Five of eleven lines meet, fewer than half: the meets of the paragraph do not
        // count. Two of four, half (a column of amounts with an amount on every row of a
        // table and on every line between two rows): they count.
        let mut met = vec![
            true, false, true, false, true, false, true, false, true, false, false,
        ];
        let ys: Vec<f32> = (0..11).map(|n| 248.5 - 7.7 * n as f32).collect();
        assert_eq!(unsupported(&ys, &mut met), 11);
        assert!(met.iter().all(|&m| !m));
        let mut met = vec![true, false, true, false];
        assert_eq!(unsupported(&ys[..4], &mut met), 0);
        assert_eq!(met, [true, false, true, false]);
        // The amounts of a band table: four lines at one pitch, the first and the third on
        // the two rows of the table. The count is at its limit (2 of 4), the column is
        // all one run, and it stays a column of the rows.
        let amounts = at(&[700.0, 686.0, 672.0, 658.0]);
        assert!(!offset_baselines(&at(&[700.0, 672.0]), &amounts, 6.7));
        // A run of two lines is no paragraph (`LONE_BLOCK`): the first line of a wrapped
        // cell on its row counts. A third line at the pitch makes it one.
        let mut met = vec![true, false];
        unsupported(&[700.0, 692.0], &mut met);
        assert_eq!(met, [true, false]);
        let mut met = vec![true, false, false];
        unsupported(&[700.0, 692.0, 684.0], &mut met);
        assert_eq!(met, [false, false, false]);
        // A step off the pitch by more than `LONE_PITCH` ends the run: 8 and 9.4 (17.5 %)
        // are two steps, 8 and 9 (12.5 %) one pitch.
        let mut met = vec![true, false, false];
        unsupported(&[700.0, 692.0, 682.6], &mut met);
        assert_eq!(met, [true, false, false]);
        let mut met = vec![true, false, false];
        unsupported(&[700.0, 692.0, 683.0], &mut met);
        assert_eq!(met, [false, false, false]);
        // Two paragraphs share the line between them (review R4 of Task 31b): three
        // lines at a pitch of 8, then from the third on four lines at a pitch of 12. Of
        // the first only the shared line meets a row (1 of 3: not supported), of the
        // second the shared line and one more (2 of 4, half: supported). Each is judged
        // by the meets as found: the first gives up its meets, the shared line with
        // them, and the second keeps the meet of its own line. Judged on the meets the
        // first had cleared, the second would stand at 1 of 4 and lose it too.
        let ys = [700.0, 692.0, 684.0, 672.0, 660.0, 648.0];
        let found = [false, false, true, false, true, false];
        let mut met = found.to_vec();
        assert_eq!(unsupported(&ys, &mut met), 3);
        assert_eq!(met, [false, false, false, false, true, false]);
        // The order of the paragraphs does not matter: the same two the other way up,
        // the one of four lines first, read alike. Nor does the order the lines are
        // given in.
        let up: Vec<f32> = ys.iter().rev().map(|y| 1348.0 - y).collect();
        let mut met: Vec<bool> = found.iter().rev().copied().collect();
        assert_eq!(unsupported(&up, &mut met), 3);
        assert_eq!(met, [false, true, false, false, false, false]);
        let turned: Vec<f32> = ys.iter().rev().copied().collect();
        let mut met: Vec<bool> = found.iter().rev().copied().collect();
        assert_eq!(unsupported(&turned, &mut met), 3);
        assert_eq!(met, [false, true, false, false, false, false]);
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
            number: false,
            boxed: None,
            rows: 1,
            trail: 0.0,
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

    /// Two columns of prose over and under a block between two rules: a row of four titles
    /// right over the upper rule, then three rows of four cells at the same edges, two of
    /// them left of the page's gutter and two right of it. Gives the pieces, the two rules
    /// and the indices of the first cell of the header and of each row.
    fn rows_between_rules() -> (Vec<Piece>, Vec<Line>, Vec<usize>) {
        let mut ps = Vec::new();
        let mut line = 0;
        let prose = |ps: &mut Vec<Piece>, top: f32, line: &mut u32| {
            for k in 0..6 {
                let y = top - 12.0 * k as f32;
                ps.push(text(30.0, 200.0, *line, y, 10.0, 40));
                ps.push(text(400.0, 560.0, *line, y, 10.0, 40));
                *line += 1;
            }
        };
        prose(&mut ps, 800.0, &mut line);
        let mut firsts = Vec::new();
        let cells = [(32.0, 50.0), (64.0, 150.0), (470.0, 506.0), (530.0, 560.0)];
        for y in [700.0, 680.0, 660.0, 640.0] {
            firsts.push(ps.len());
            for (x0, x1) in cells {
                ps.push(text(x0, x1, line, y, 10.0, 6));
            }
            line += 1;
        }
        prose(&mut ps, 600.0, &mut line);
        let rule = |at: f32| Line {
            at,
            from: 30.0,
            to: 560.0,
        };
        (ps, vec![rule(696.0), rule(630.0)], firsts)
    }

    #[test]
    fn rows_under_a_rule_in_pieces_are_read_row_by_row() {
        let (ps, rules, firsts) = rows_between_rules();
        let whole = |cut: &Cut| {
            let order = cut.groups.concat();
            let at = |i: usize| order.iter().position(|&o| o == i).unwrap();
            firsts
                .iter()
                .all(|&f| (1..4).all(|c| at(f + c) == at(f) + c))
        };
        // The gutter of the page parts every row: the left cells of all rows read before
        // the right ones.
        let plain = cut_of(&ps, &rules);
        assert_whole(&plain, ps.len());
        assert!(!whole(&plain), "{:?}", plain.groups);
        // The upper rule is drawn in pieces whose joints stand between the cells: the rows
        // are rows of one table, each read whole, the header row with them.
        let joints = [(696.0, 57.0), (696.0, 460.0), (696.0, 518.0)];
        let ruled = Ruled {
            rules: &rules,
            joined: &rules[..1],
            joints: &joints,
        };
        let cut = xy_cut_ruled(&ps, &ruled, &Limits::default(), &|| false);
        assert_whole(&cut, ps.len());
        assert!(whole(&cut), "{:?}", cut.groups);
        // A joint over a cell marks no column of these rows: the gutter parts them as before.
        let over_a_cell = [(696.0, 100.0)];
        let ruled = Ruled {
            joints: &over_a_cell,
            ..ruled
        };
        let cut = xy_cut_ruled(&ps, &ruled, &Limits::default(), &|| false);
        assert_eq!(cut.groups, plain.groups);
    }

    #[test]
    fn rows_without_titles_over_the_rule_stay_in_their_columns() {
        // The same rows with a line of prose over the upper rule and no row of titles: the
        // joints alone do not make them a table.
        let (mut ps, rules, firsts) = rows_between_rules();
        for c in (0..4).rev() {
            ps.remove(firsts[0] + c);
        }
        let plain = cut_of(&ps, &rules);
        let joints = [(696.0, 57.0), (696.0, 460.0), (696.0, 518.0)];
        let ruled = Ruled {
            rules: &rules,
            joined: &rules[..1],
            joints: &joints,
        };
        let cut = xy_cut_ruled(&ps, &ruled, &Limits::default(), &|| false);
        assert_eq!(cut.groups, plain.groups);
    }

    #[test]
    fn a_header_with_its_baseline_on_the_rule_is_a_header() {
        // Review N4 of Task 31b. The titles sit on the rule (baseline 700): the middle of
        // the rule a tenth of the size over the baseline is on it still (a title on its
        // underline, rounded or jittered); three tenths over the baseline it runs through
        // the titles, and they are no header over it.
        let (ps, rules, firsts) = rows_between_rules();
        for (at, header) in [(701.0, true), (700.0, true), (703.0, false)] {
            let rules = [Line { at, ..rules[0] }, rules[1]];
            let joints = [(at, 57.0), (at, 460.0), (at, 518.0)];
            let ruled = Ruled {
                rules: &rules,
                joined: &rules[..1],
                joints: &joints,
            };
            let cut = xy_cut_ruled(&ps, &ruled, &Limits::default(), &|| false);
            assert_whole(&cut, ps.len());
            let order = cut.groups.concat();
            let at_of = |i: usize| order.iter().position(|&o| o == i).unwrap();
            let whole = firsts
                .iter()
                .all(|&f| (1..4).all(|c| at_of(f + c) == at_of(f) + c));
            assert_eq!(whole, header, "rule at {at}: {:?}", cut.groups);
        }
    }

    #[test]
    fn years_and_numbers_over_the_rule_are_titles() {
        // Review N5 of Task 31b. Titles that are whole numbers (years over columns of
        // amounts, the numbers of columns) are a header; one amount among them is a row of
        // values, no header.
        let (mut ps, rules, firsts) = rows_between_rules();
        let plain = cut_of(&ps, &rules);
        let joints = [(696.0, 57.0), (696.0, 460.0), (696.0, 518.0)];
        let ruled = Ruled {
            rules: &rules,
            joined: &rules[..1],
            joints: &joints,
        };
        for c in 2..4 {
            ps[firsts[0] + c].value = true;
            ps[firsts[0] + c].number = true;
        }
        let cut = xy_cut_ruled(&ps, &ruled, &Limits::default(), &|| false);
        assert_ne!(cut.groups, plain.groups);
        ps[firsts[0] + 3].number = false;
        let cut = xy_cut_ruled(&ps, &ruled, &Limits::default(), &|| false);
        assert_eq!(cut.groups, plain.groups);
        // Review P4 of Task 31b. One whole number in the row is the amount of the label
        // beside it (`Total 250`), no title: a row of values, no header. Two are titles.
        for c in 2..4 {
            ps[firsts[0] + c].value = c == 3;
            ps[firsts[0] + c].number = c == 3;
        }
        let cut = xy_cut_ruled(&ps, &ruled, &Limits::default(), &|| false);
        assert_eq!(cut.groups, plain.groups);
        ps[firsts[0] + 2].value = true;
        ps[firsts[0] + 2].number = true;
        let cut = xy_cut_ruled(&ps, &ruled, &Limits::default(), &|| false);
        assert_ne!(cut.groups, plain.groups);
        assert!(reads_as_number(" 2025 ") && reads_as_number("3"));
        assert!(!reads_as_number("12,50") && !reads_as_number("Q1") && !reads_as_number(" "));
    }

    #[test]
    fn two_whole_amounts_over_the_rule_are_titles_still() {
        // A pin of a known gap, not of a wanted read (review Q6 of Task 31b, owner Task
        // 26). The row right over the rule in pieces is a row of sums, `Total` and two
        // whole-number amounts in the columns of the rows under the rule (two counts, or
        // amounts in a currency without decimals). The cut sees two whole numbers side by
        // side, which are titles as two years are (see `TITLE_NUMBERS`), and takes the row
        // for the header of the rows under the rule: the sums leave their block and stand
        // as titles over the columns. Nothing in a whole number tells an amount from a
        // year. Task 26 reads the amounts of labels and values; when it tells this row
        // from a row of titles, the cut here is the plain one, and this pin is changed.
        let (mut ps, rules, firsts) = rows_between_rules();
        let plain = cut_of(&ps, &rules);
        let joints = [(696.0, 57.0), (696.0, 460.0), (696.0, 518.0)];
        let ruled = Ruled {
            rules: &rules,
            joined: &rules[..1],
            joints: &joints,
        };
        for c in 2..4 {
            ps[firsts[0] + c].value = true;
            ps[firsts[0] + c].number = true;
        }
        let cut = xy_cut_ruled(&ps, &ruled, &Limits::default(), &|| false);
        assert_ne!(
            cut.groups, plain.groups,
            "the known gap is closed: turn this pin"
        );
        // With a decimal amount among them the row is a row of values already.
        ps[firsts[0] + 3].number = false;
        let cut = xy_cut_ruled(&ps, &ruled, &Limits::default(), &|| false);
        assert_eq!(cut.groups, plain.groups);
    }

    #[test]
    fn a_line_of_prose_right_over_the_rule_is_no_header() {
        // One piece of prose close over the upper rule, where the titles stood: no header,
        // so the rows stay in the columns of the page.
        let (mut ps, rules, firsts) = rows_between_rules();
        for c in (1..4).rev() {
            ps.remove(firsts[0] + c);
        }
        ps[firsts[0]].bounds.x0 = 30.0;
        ps[firsts[0]].bounds.x1 = 200.0;
        ps[firsts[0]].chars = 40;
        let plain = cut_of(&ps, &rules);
        let joints = [(696.0, 57.0), (696.0, 460.0), (696.0, 518.0)];
        let ruled = Ruled {
            rules: &rules,
            joined: &rules[..1],
            joints: &joints,
        };
        let cut = xy_cut_ruled(&ps, &ruled, &Limits::default(), &|| false);
        assert_eq!(cut.groups, plain.groups);
    }

    #[test]
    fn lines_of_a_label_and_its_amount_between_rules_are_no_item_rows() {
        // Totals under a table: the line over the upper rule holds an amount, so it is no
        // row of titles, and the lines under the rule are not looked at alone. The same holds
        // for a row of titles with fewer pieces than the rows need.
        let (mut ps, rules, firsts) = rows_between_rules();
        let plain = cut_of(&ps, &rules);
        let joints = [(696.0, 57.0), (696.0, 460.0), (696.0, 518.0)];
        let ruled = Ruled {
            rules: &rules,
            joined: &rules[..1],
            joints: &joints,
        };
        ps[firsts[0] + 3].value = true;
        let cut = xy_cut_ruled(&ps, &ruled, &Limits::default(), &|| false);
        assert_eq!(cut.groups, plain.groups);
        // The same when the amount is a whole number (a count, an amount in a currency
        // without decimals): one number beside labels is no title.
        ps[firsts[0] + 3].number = true;
        let cut = xy_cut_ruled(&ps, &ruled, &Limits::default(), &|| false);
        assert_eq!(cut.groups, plain.groups);
        ps[firsts[0] + 3].number = false;
        ps[firsts[0] + 3].value = false;
        let cut = xy_cut_ruled(&ps, &ruled, &Limits::default(), &|| false);
        assert_ne!(cut.groups, plain.groups);
        // Two titles over rows of four cells: too few.
        ps.remove(firsts[0] + 2);
        ps.remove(firsts[0] + 1);
        let plain = cut_of(&ps, &rules);
        let cut = xy_cut_ruled(&ps, &ruled, &Limits::default(), &|| false);
        assert_eq!(cut.groups, plain.groups);
    }

    #[test]
    fn a_line_that_reaches_into_the_gutter_stays_in_its_column() {
        // Two columns; one line of the left column is longer and ends in the white space
        // between them, and one line of the right column starts in it. Neither is a piece
        // across: each column reads whole, the left one first.
        let mut ps = Vec::new();
        for k in 0..8u32 {
            let y = 700.0 - 12.0 * k as f32;
            let left_end = if k == 3 { 262.0 } else { 240.0 };
            let right_start = if k == 5 { 268.0 } else { 300.0 };
            ps.push(text(30.0, left_end, k, y, 10.0, 40));
            ps.push(text(right_start, 560.0, k, y, 10.0, 40));
        }
        let cut = cut_of(&ps, &[]);
        assert_whole(&cut, ps.len());
        let left: Vec<usize> = (0..8).map(|k| 2 * k).collect();
        let right: Vec<usize> = (0..8).map(|k| 2 * k + 1).collect();
        assert_eq!(cut.groups, vec![left, right]);
    }

    /// The first page of a letter, as the cut of a real page printed it (places only): five
    /// head lines of 9 points set flush right at 565 (rows 0 to 4, the third the longest), a
    /// sender line, an address set in, a date and a subject with a box of two lines on the
    /// right, the letter with lines of every length (row 22 runs across the page, rows 26
    /// to 28 nearly), rows of an item table and two lines of totals. Each piece: row, left,
    /// right, baseline, size, ink.
    fn letter_page() -> Vec<Piece> {
        let places: [(u32, f32, f32, f32, f32, bool); 55] = [
            (0, 494.0, 565.0, 808.0, 9.0, true),
            (1, 470.1, 565.0, 799.0, 9.0, true),
            (2, 401.6, 565.0, 790.0, 9.0, true),
            (3, 481.9, 565.0, 781.0, 9.0, true),
            (4, 431.5, 565.0, 772.0, 9.0, true),
            (5, 30.0, 248.4, 750.0, 9.0, true),
            (6, 30.0, 32.5, 738.0, 11.0, false),
            (7, 30.0, 32.5, 726.0, 11.0, false),
            (8, 30.0, 32.5, 714.0, 11.0, false),
            (9, 56.9, 112.6, 699.0, 11.0, true),
            (10, 56.9, 129.0, 686.0, 11.0, true),
            (11, 56.9, 131.0, 673.0, 11.0, true),
            (12, 30.0, 32.5, 657.0, 11.0, false),
            (13, 30.0, 32.5, 645.0, 11.0, false),
            (14, 30.0, 32.5, 633.0, 11.0, false),
            (15, 32.0, 82.1, 622.0, 11.0, true),
            (15, 465.7, 563.0, 622.0, 11.0, true),
            (16, 32.0, 194.6, 609.0, 11.0, true),
            (16, 458.0, 565.0, 609.0, 11.0, true),
            (17, 30.0, 33.2, 595.0, 14.0, false),
            (18, 30.0, 125.0, 583.0, 11.0, true),
            (19, 30.0, 32.5, 571.0, 11.0, false),
            (20, 30.0, 169.7, 559.0, 11.0, true),
            (21, 30.0, 369.0, 547.0, 11.0, true),
            (22, 30.0, 564.7, 535.0, 11.0, true),
            (23, 30.0, 230.9, 523.0, 11.0, true),
            (24, 469.0, 554.0, 506.0, 11.0, true),
            (25, 32.0, 402.5, 496.0, 11.0, true),
            (26, 30.0, 550.6, 422.0, 11.0, true),
            (27, 30.0, 519.2, 410.0, 11.0, true),
            (28, 30.0, 559.0, 398.0, 11.0, true),
            (29, 30.0, 391.7, 386.0, 11.0, true),
            (30, 30.0, 32.5, 374.0, 11.0, false),
            (31, 32.0, 50.5, 361.0, 11.0, true),
            (31, 64.1, 115.5, 361.0, 11.0, true),
            (31, 472.9, 504.2, 361.0, 11.0, true),
            (31, 541.2, 563.0, 361.0, 11.0, true),
            (32, 43.3, 48.8, 344.0, 11.0, true),
            (32, 64.1, 151.0, 344.0, 11.0, true),
            (32, 500.6, 506.1, 344.0, 11.0, true),
            (32, 537.5, 565.0, 344.0, 11.0, true),
            (33, 64.1, 194.6, 333.0, 11.0, true),
            (34, 44.3, 49.8, 309.0, 11.0, true),
            (34, 64.1, 250.6, 309.0, 11.0, true),
            (34, 500.6, 506.1, 309.0, 11.0, true),
            (34, 532.0, 565.0, 309.0, 11.0, true),
            (35, 64.1, 194.9, 298.0, 11.0, true),
            (36, 44.3, 49.8, 274.0, 11.0, true),
            (36, 64.1, 148.0, 274.0, 11.0, true),
            (36, 500.6, 506.1, 274.0, 11.0, true),
            (36, 537.5, 565.0, 274.0, 11.0, true),
            (37, 64.1, 194.9, 263.0, 11.0, true),
            (38, 431.6, 506.1, 239.0, 11.0, true),
            (38, 532.0, 565.0, 239.0, 11.0, true),
            (39, 473.1, 565.0, 222.0, 11.0, true),
        ];
        places
            .into_iter()
            .map(|(line, x0, x1, y, size, inked)| Piece {
                inked,
                ..text(x0, x1, line, y, size, ((x1 - x0) / (0.5 * size)) as usize)
            })
            .collect()
    }

    /// The gutter of `ps` with its sides, as the cut finds it.
    fn gutter_of(ps: &[Piece]) -> (Sides, Why) {
        let ids: Vec<usize> = (0..ps.len()).collect();
        gutter(ps, &ids).expect("a gutter")
    }

    /// The piece of row `line` of `ps` that reaches furthest.
    fn row(ps: &[Piece], line: u32) -> &Piece {
        ps.iter()
            .filter(|p| p.line == line)
            .max_by(|a, b| (a.bounds.x1 - a.bounds.x0).total_cmp(&(b.bounds.x1 - b.bounds.x0)))
            .expect("a row")
    }

    /// Whether the head lines (rows 0 to 4) of [`letter_page`] read one after the other.
    fn head_whole(ps: &[Piece]) -> bool {
        let cut = cut_of(ps, &[]);
        assert_whole(&cut, ps.len());
        let order = cut.groups.concat();
        let at = order.iter().position(|&o| o == 0).unwrap();
        order[at..].starts_with(&[0, 1, 2, 3, 4])
    }

    #[test]
    fn the_longest_line_of_a_block_set_flush_right_is_a_line_of_its_block() {
        // The path of the real page: the gutter is found at a level that a few lines cover,
        // and its left edge falls a point or two right of where the longest head line starts.
        let ps = letter_page();
        let (sides, why) = gutter_of(&ps);
        assert_eq!(why, Why::Offset);
        assert!((sides.g0 - 403.1).abs() < 0.5 && (sides.g1 - 430.6).abs() < 0.5);
        assert!(row(&ps, 2).bounds.x0 < sides.g0 && row(&ps, 2).bounds.x1 > sides.g1);
        // The line ends where the lines over and under it end and starts before the end of
        // one line of the left side only: a line of the right side, with its block.
        assert_eq!(sides.place(row(&ps, 2)), Place::Right);
        assert!(head_whole(&ps));
        // The lines of the letter that reach from its left edge into most rows of the right
        // side stand across, also the ragged ones that end before the right edge.
        for line in [22, 26, 27, 28] {
            assert_eq!(sides.place(row(&ps, line)), Place::Across, "row {line}");
        }
        // The same block, its longest line set further left: 30 points over the ends of a
        // few more lines of the letter, still not over most rows of the left side.
        let mut wider = ps.clone();
        wider[2].bounds.x0 = 371.6;
        let (sides, _) = gutter_of(&wider);
        assert_eq!(sides.place(&wider[2]), Place::Right);
        assert!(head_whole(&wider));
        // The other way: a line from the left edge of the letter to the right edge of the
        // block reaches over both sides and stands across, also between the head lines.
        let mut across = ps.clone();
        across[2].bounds.x0 = 30.0;
        let (sides, _) = gutter_of(&across);
        assert_eq!(sides.place(&across[2]), Place::Across);
        assert!(!head_whole(&across));
    }

    #[test]
    fn a_ragged_line_that_runs_past_the_gutter_is_a_line_of_its_side() {
        // The letter line of row 25 ends 9 points further right than on the real page: the
        // left edge of the gutter moves with it, so the longest head line starts well
        // before that edge. Both lines stay on their sides.
        let mut ps = letter_page();
        let long = ps.iter().position(|p| p.line == 25).unwrap();
        ps[long].bounds.x1 += 9.0;
        let (sides, _) = gutter_of(&ps);
        assert!(sides.g0 - ps[2].bounds.x0 > 0.8 * ps[2].size, "{sides:?}");
        assert_eq!(sides.place(&ps[2]), Place::Right);
        assert_eq!(sides.place(&ps[long]), Place::Left);
        assert!(head_whole(&ps));
        // Longer still, past the start of two rows of the right side: a line of the left
        // side, as it starts where the lines before and after it start.
        ps[long].bounds.x1 = 440.0;
        let (sides, _) = gutter_of(&ps);
        assert!(sides.reaches(&ps[long]));
        assert_eq!(sides.place(&ps[long]), Place::Left);
        assert!(head_whole(&ps));
        // The other way: the line runs on into most rows of the right side. It stands over
        // both sides, a line across.
        ps[long].bounds.x1 = 520.0;
        let (sides, _) = gutter_of(&ps);
        assert_eq!(sides.place(&ps[long]), Place::Across);
    }

    /// Two blocks side by side for [`Sides::place`]: eight rows of the left side that end
    /// at 250 (one at `long`), and eight rows of the right side at 9 points that start at 480
    /// and end at 565, on baselines of their own. Row 20 is free for the piece asked about,
    /// and so is the baseline 590, under all rows.
    fn two_blocks_to(long: f32) -> Vec<Piece> {
        let mut ps = Vec::new();
        for k in 0..8u32 {
            let y = 700.0 - 12.0 * k as f32;
            let end = if k == 6 { long } else { 250.0 };
            ps.push(text(30.0, end, 2 * k, y, 10.0, 40));
            ps.push(text(480.0, 565.0, 2 * k + 1, y - 5.0, 9.0, 18));
        }
        ps
    }

    fn two_blocks() -> Vec<Piece> {
        two_blocks_to(310.0)
    }

    /// Where the piece `q` stands among the pieces `ps`, at the gutter from `g0` to 480.
    fn stands_in(mut ps: Vec<Piece>, q: Piece, g0: f32) -> Place {
        ps.push(q);
        let ids: Vec<usize> = (0..ps.len()).collect();
        let sides = Sides::new(&ps, &ids, (g0, 480.0), GUTTER_SLACK * 9.0);
        sides.place(&ps[ps.len() - 1])
    }

    /// Where the piece from `x0` to `x1` at 9 points, on the baseline `y` as row `line`,
    /// stands at the gutter from 310 to 480 of [`two_blocks`].
    fn stands(x0: f32, x1: f32, line: u32, y: f32) -> Place {
        stands_in(two_blocks(), text(x0, x1, line, y, 9.0, 30), 310.0)
    }

    #[test]
    fn a_piece_stands_over_a_side_by_most_of_its_rows() {
        // From the left edge to the right edge: over all rows of both sides.
        assert_eq!(stands(30.0, 565.0, 20, 590.0), Place::Across);
        // Set in on the left and ragged on the right, far from the other rows: still over
        // all rows of both sides.
        assert_eq!(stands(60.0, 500.0, 20, 590.0), Place::Across);
        // It starts after the end of seven of the eight rows of the left side: over the right
        // side only, where its larger part lies too.
        assert_eq!(stands(300.0, 565.0, 20, 590.0), Place::Right);
        // It ends before the start of the rows of the right side: a line of the left side.
        assert_eq!(stands(30.0, 470.0, 20, 590.0), Place::Left);
        // More than half of the rows decide. Five of the eight rows of the right side start
        // at 440 now: a piece that ends at 450 reaches into five of them, one that ends at
        // 436 into none.
        let place = |x1: f32| {
            let mut ps = two_blocks();
            for p in ps.iter_mut().filter(|p| p.line % 2 == 1 && p.line < 10) {
                p.bounds.x0 = 440.0;
            }
            ps.push(text(30.0, x1, 20, 590.0, 9.0, 30));
            let ids: Vec<usize> = (0..ps.len()).collect();
            let sides = Sides::new(&ps, &ids, (310.0, 440.0), GUTTER_SLACK * 9.0);
            sides.place(&ps[ps.len() - 1])
        };
        assert_eq!(place(450.0), Place::Across);
        assert_eq!(place(436.0), Place::Left);
    }

    #[test]
    fn a_piece_that_shares_the_edge_of_the_row_next_to_it_is_a_line_of_that_side() {
        // The left side has one long row (to 400). A long line among the rows of the right
        // side (row 7 lies 3.5 points over it, row 9 8.5 points under it) starts at 300:
        // over the right side by its rows, and into the long row of the left side. It
        // ends where the rows next to it end: a line of their block, whatever its start.
        let y = 643.5;
        let place = |x1: f32, line: u32, y: f32| {
            stands_in(
                two_blocks_to(400.0),
                text(300.0, x1, line, y, 9.0, 30),
                400.0,
            )
        };
        assert_eq!(place(565.0, 8, y), Place::Right);
        // Within a quarter of its size (2.25 points at 9 points) the edge is shared.
        assert_eq!(place(563.2, 8, y), Place::Right);
        // Further off it shares no edge. It reaches into the one long row of the left side
        // and into the rows of the right side: no row says where it stands.
        assert_eq!(place(561.4, 8, y), Place::Unsure);
        assert_eq!(place(568.6, 8, y), Place::Unsure);
        // The rows of the block follow each other: a row more than four sizes away is no
        // row next to it. Under the last row of the right side (baseline 611): 30 points
        // under it the edge is shared, 40 points under it not.
        assert_eq!(place(565.0, 20, 581.0), Place::Right);
        assert_eq!(place(565.0, 20, 571.0), Place::Unsure);
        // The mirror: the longest line of the block on the left runs past the gutter and
        // the start of no row of the right side. It starts where the rows next to it start.
        let mut ps = two_blocks();
        ps[12].bounds.x1 = 500.0;
        let ids: Vec<usize> = (0..ps.len()).collect();
        let sides = Sides::new(&ps, &ids, (250.0, 480.0), GUTTER_SLACK * 9.0);
        assert_eq!(sides.place(&ps[12]), Place::Across);
        ps[12].bounds.x1 = 478.0;
        let sides = Sides::new(&ps, &ids, (250.0, 470.0), GUTTER_SLACK * 9.0);
        assert_eq!(sides.place(&ps[12]), Place::Left);
    }

    #[test]
    fn a_piece_that_no_row_supports_is_unsure() {
        // Design ruling of fix round 5 of Task 31b. The ink of the sides ends at 310 (one
        // long row; the others end at 250) and starts at 480. A piece that reaches into
        // the gutter with no edge of a row next to it, and without the rows of one side
        // alone under it, has no proven place: a heading centred in the white space or
        // over its edges, a line off its middle, the number or mark of a box. Nothing
        // else is asked: not where its middle or its larger part lies, not whether it holds
        // a word, not what stands on its baseline.
        for (x0, x1) in [
            (360.0, 430.0),
            (300.0, 490.0),
            (366.0, 436.0),
            (354.0, 424.0),
            (320.0, 400.0),
        ] {
            assert_eq!(stands(x0, x1, 20, 590.0), Place::Unsure, "{x0} {x1}");
            assert_eq!(stands(x0, x1, 0, 700.0), Place::Unsure, "{x0} {x1}");
        }
        let mark = Piece {
            ..text(360.0, 430.0, 20, 590.0, 9.0, 2)
        };
        assert_eq!(stands_in(two_blocks(), mark, 310.0), Place::Unsure);
        // The rows of one side alone prove that side. From 400 to 500 the piece starts
        // after the end of every row of the left side and reaches into all rows of the
        // right side: a line of the right side.
        assert_eq!(stands(400.0, 500.0, 20, 590.0), Place::Right);
        // The other way: from 305 it starts before the end of the long row of the left
        // side. One row of the other side under it, and the rows no longer say.
        assert_eq!(stands(305.0, 500.0, 20, 590.0), Place::Unsure);
        // The mirror: over the rows of the left side and short of the right side it is a
        // line of the left side; into one row of the right side (set to start at 440) it
        // is unsure, into most of them across.
        let place = |starts: usize| {
            let mut ps = two_blocks();
            for p in ps.iter_mut().filter(|p| p.line % 2 == 1).take(starts) {
                p.bounds.x0 = 440.0;
            }
            ps.push(text(100.0, 470.0, 20, 590.0, 9.0, 30));
            let ids: Vec<usize> = (0..ps.len()).collect();
            let sides = Sides::new(&ps, &ids, (310.0, 440.0), GUTTER_SLACK * 9.0);
            sides.place(&ps[ps.len() - 1])
        };
        assert_eq!(
            (place(0), place(1), place(5)),
            (Place::Left, Place::Unsure, Place::Across)
        );
    }

    #[test]
    fn a_piece_that_leaves_a_gutter_free_is_a_line_of_the_side_it_reaches() {
        // The path of a public form: a value under its label, a little longer than the
        // label, in a block whose other rows are short. It reaches into the one long row
        // of the left side (to 310) and into no row of the right side (from 480): counted
        // as a row of the left side, the white space up to the right side is still a
        // gutter, which is at least 0.8 sizes wide (7.2 points at 9 points).
        assert_eq!(stands(280.0, 400.0, 20, 590.0), Place::Left);
        assert_eq!(stands(280.0, 472.0, 20, 590.0), Place::Left);
        // The other way: 6 points before the right side no gutter is left, and from 320,
        // after the end of the long row, it reaches into no row of the left side.
        assert_eq!(stands(280.0, 474.0, 20, 590.0), Place::Unsure);
        assert_eq!(stands(320.0, 400.0, 20, 590.0), Place::Unsure);
        // The mirror: one row of the right side starts at 440. A piece that ends in it
        // (at 450) and starts a gutter's width after the long row of the left side is a
        // line of the right side; 6 points after it, its place is unsure.
        let place = |x0: f32| {
            let mut ps = two_blocks();
            ps[1].bounds.x0 = 440.0;
            ps.push(text(x0, 450.0, 20, 590.0, 9.0, 30));
            let ids: Vec<usize> = (0..ps.len()).collect();
            let sides = Sides::new(&ps, &ids, (310.0, 440.0), GUTTER_SLACK * 9.0);
            sides.place(&ps[ps.len() - 1])
        };
        assert_eq!((place(318.0), place(316.0)), (Place::Right, Place::Unsure));
    }

    /// Where the piece from `x0` to `x1` at 9 points stands at the gutter from 310 to 480
    /// of [`two_blocks`], on a row under all rows of the blocks that it shares with the
    /// pieces `beside` (see [`places`]).
    fn stands_beside(x0: f32, x1: f32, beside: &[(f32, f32)]) -> Place {
        let mut ps = two_blocks();
        ps.extend(beside.iter().map(|&(a, b)| text(a, b, 20, 590.0, 9.0, 1)));
        ps.push(text(x0, x1, 20, 590.0, 9.0, 30));
        let ids: Vec<usize> = (0..ps.len()).collect();
        let sides = Sides::new(&ps, &ids, (310.0, 480.0), GUTTER_SLACK * 9.0);
        places(&ps, &ids, &sides)[ps.len() - 1]
    }

    #[test]
    fn a_piece_near_one_side_is_a_piece_of_it_beside_other_ink_of_its_row() {
        // The path of a public form (irs-f6744 untagged page 108): an amount set flush
        // right in its box, in the white space between the sides, in the ink of no row,
        // with its currency sign at the left of its row. It starts 3 points after the
        // long row of the left side (less than a gutter's width, 0.8 sizes: 7.2 points at
        // 9) and leaves a gutter free before the right side: counted as a piece of the
        // left side the gutter stands, counted as one of the right side there is none.
        // The mirror for the right side.
        let sign = [(200.0, 205.0)];
        assert_eq!(stands_beside(313.0, 400.0, &sign), Place::Left);
        assert_eq!(stands_beside(313.0, 472.0, &sign), Place::Left);
        assert_eq!(stands_beside(400.0, 476.0, &sign), Place::Right);
        assert_eq!(stands_beside(318.0, 476.0, &sign), Place::Right);
        // The other ink of the row may stand on either side, or on both.
        let line = [(500.0, 540.0)];
        assert_eq!(stands_beside(313.0, 400.0, &line), Place::Left);
        assert_eq!(stands_beside(400.0, 476.0, &line), Place::Right);
        let both = [(200.0, 205.0), (500.0, 540.0)];
        assert_eq!(stands_beside(313.0, 400.0, &both), Place::Left);
        // The other way: a gutter's width free on both sides (8 points after the long
        // row), or on neither (6 points before the right side): nothing says whose it is.
        assert_eq!(stands_beside(318.0, 400.0, &sign), Place::Unsure);
        assert_eq!(stands_beside(318.0, 472.0, &sign), Place::Unsure);
        assert_eq!(stands_beside(313.0, 474.0, &sign), Place::Unsure);
        // The width is a step at 0.8 sizes (review R6 of Task 31b), moved both ways by a
        // tenth of a point, about 1 % of the size. Before the right side: 7.3 points free
        // is a gutter, 7.1 points is none (free on neither side). After the left side:
        // 7.1 points is less than a gutter, 7.3 points is one (free on both sides).
        assert_eq!(stands_beside(313.0, 472.7, &sign), Place::Left);
        assert_eq!(stands_beside(313.0, 472.9, &sign), Place::Unsure);
        assert_eq!(stands_beside(317.1, 400.0, &sign), Place::Left);
        assert_eq!(stands_beside(317.3, 400.0, &sign), Place::Unsure);
        // The other way (review R1 of Task 31b): the same pieces alone on their row. The
        // ink of the sides is one row each and says nothing of a row of its own: near a
        // side or not, the place is unsure, and the rows over and under the piece decide
        // (see `a_lone_piece_in_the_gutter_reads_after_both_columns`). Other ink without
        // a proven side (a second piece in the white space) is no ink of a line either.
        for (x0, x1) in [(313.0, 400.0), (313.0, 472.0), (400.0, 476.0)] {
            assert_eq!(stands_beside(x0, x1, &[]), Place::Unsure, "{x0} {x1}");
            assert_eq!(stands(x0, x1, 20, 590.0), Place::Unsure, "{x0} {x1}");
        }
        assert_eq!(
            stands_beside(313.0, 380.0, &[(390.0, 400.0)]),
            Place::Unsure
        );
    }

    #[test]
    fn a_heading_centred_on_the_page_over_unequal_columns_stands_across() {
        // Review P6 of Task 31b, the geometry of a public form: a heading from 189.4 to
        // 422.6 on a page 612 points wide, over a wide left block and a narrow right one
        // with the gutter from 359.5 to 405.5.
        let page = |heading: (f32, f32)| {
            let mut ps = vec![
                text(40.0, 572.0, 0, 760.0, 10.0, 90),
                text(40.0, 572.0, 1, 748.0, 10.0, 90),
                text(heading.0, heading.1, 2, 724.0, 12.0, 40),
            ];
            let ends = [359.5, 310.0, 342.0, 296.0, 351.0, 330.0];
            for k in 0..6u32 {
                let y = 700.0 - 12.0 * k as f32;
                ps.push(text(40.0, ends[k as usize], 3 + 2 * k, y, 10.0, 50));
                ps.push(text(405.5, 572.0, 4 + 2 * k, y - 5.0, 10.0, 28));
            }
            ps
        };
        let ps = page((189.4, 422.6));
        let ids: Vec<usize> = (0..ps.len()).collect();
        let sides = Sides::new(&ps, &ids, (359.5, 405.5), GUTTER_SLACK * 10.0);
        // It starts before every row of the left block ends and ends after every row of
        // the right block starts: over both sides by their rows, a piece across. It reads
        // after the lines across over it and before both blocks under it.
        assert_eq!(sides.place(&ps[2]), Place::Across);
        let cut = cut_of(&ps, &[]);
        assert_whole(&cut, ps.len());
        let at = cut.groups.iter().position(|g| g == &vec![2]);
        assert_eq!(at, Some(1), "{:?}", cut.groups);
        // The other way: the same line set to the left, an indented line of the left block
        // that runs into the gutter and into no row of the right block.
        let ps = page((60.0, 400.0));
        let sides = Sides::new(&ps, &ids, (359.5, 405.5), GUTTER_SLACK * 10.0);
        assert_eq!(sides.place(&ps[2]), Place::Left);
    }

    /// The page of review P2 of Task 31b: a left column of eight lines from 30 to 250 at 10
    /// points, a ninth line between them from 100 to `x1`, a right block of nine short
    /// lines from 480 to 565 at 9 points on baselines of their own, and with `margin` a
    /// short piece that starts there, lower on the page.
    fn line_to_the_right_block(x1: f32, margin: Option<f32>) -> Vec<Piece> {
        let mut ps = Vec::new();
        for k in 0..9u32 {
            let y = 700.0 - 12.0 * k as f32;
            let (x0, end) = if k == 4 { (100.0, x1) } else { (30.0, 250.0) };
            ps.push(text(x0, end, 2 * k, y, 10.0, 40));
            ps.push(text(480.0, 565.0, 2 * k + 1, y - 5.0, 9.0, 18));
        }
        if let Some(x) = margin {
            ps.push(text(x, x + 20.0, 99, 550.0, 10.0, 4));
        }
        ps
    }

    #[test]
    fn a_line_that_ends_where_the_other_side_starts_stands_alike_at_any_edge() {
        // Review P2 of Task 31b. The line ends a tenth of a point before or after the start
        // of the right block, where the edge of the gutter lies: the edge is an edge of the
        // steps the band is measured in and is not asked. The line reaches into no row of
        // the right block: a line of the left side, and both columns read whole.
        let left: Vec<usize> = (0..9).map(|k| 2 * k).collect();
        let right: Vec<usize> = (0..9).map(|k| 2 * k + 1).collect();
        for x1 in [479.9, 480.0, 480.1] {
            let ps = line_to_the_right_block(x1, None);
            let (sides, _) = gutter_of(&ps);
            assert_eq!(sides.place(&ps[8]), Place::Left, "{x1}");
            assert_eq!(cut_of(&ps, &[]).groups, vec![left.clone(), right.clone()]);
        }
        // Ink in the margin, a point further left or not, moves the steps and with them the
        // edges of the gutter; the line stands as before.
        let mut edges = Vec::new();
        for margin in [30.0, 29.0, 28.0, 27.0, 26.0, 25.0, 24.0] {
            let ps = line_to_the_right_block(479.9, Some(margin));
            let (sides, _) = gutter_of(&ps);
            edges.push(sides.g1);
            assert_eq!(sides.place(&ps[8]), Place::Left, "{margin}");
        }
        assert!(edges.iter().any(|&g| g < 479.9) && edges.iter().any(|&g| g > 479.9));
        // The other way: the line runs on into the rows of the right block and stands over
        // both sides.
        let ps = line_to_the_right_block(500.0, None);
        let (sides, _) = gutter_of(&ps);
        assert_eq!(sides.place(&ps[8]), Place::Across);
        // Where the step is (review Q2 of the third re-review): all rows
        // of the right block start at one edge, so the line is over all of them or none.
        // It reaches into a row by more than the slack, half a step of the band measure
        // (0.6 points here): less is the rounding of two places, not ink in a row. The
        // step is a share of the text size and asks no edge of the gutter.
        for (x1, want) in [(480.4, Place::Left), (480.9, Place::Across)] {
            let ps = line_to_the_right_block(x1, None);
            let (sides, _) = gutter_of(&ps);
            assert_eq!(sides.place(&ps[8]), want, "{x1}");
        }
    }

    /// Two columns of prose of eight rows at 10 points, the right one from `right` on
    /// baselines of its own, and marks of 6 points (no words) on the rows of the left
    /// column, each from its place in `marks`.
    fn columns_with_marks(right: f32, marks: &[f32]) -> Vec<Piece> {
        let mut ps = Vec::new();
        for k in 0..8u32 {
            let y = 700.0 - 12.0 * k as f32;
            ps.push(text(30.0, 250.0, 2 * k, y, 10.0, 40));
            if let Some(&x) = marks.get(k as usize) {
                ps.push(Piece {
                    ..text(x, x + 6.0, 2 * k, y, 6.0, 2)
                });
            }
            ps.push(text(right, 565.0, 2 * k + 1, y - 5.0, 10.0, 40));
        }
        ps
    }

    #[test]
    fn marks_in_a_gutter_cross_nothing() {
        // Review P3 of Task 31b. The number of a box, a check mark or a bullet in the white
        // space between two columns is no piece across: it parts no band, it does not count
        // against the gutter, and it stays with the line of its row. Both columns read
        // whole, the left one first, whatever the number of marks. Each mark leaves a
        // gutter's width free on both sides or on neither: nothing says whose it is. (A
        // mark nearer than that to one column only, on a row with other ink, is a piece
        // of that column, see
        // `a_piece_near_one_side_is_a_piece_of_it_beside_other_ink_of_its_row`.)
        for (right, marks) in [(270.0, &[257.0][..]), (270.0, &[257.0, 257.0])] {
            let ps = columns_with_marks(right, marks);
            let (sides, _) = gutter_of(&ps);
            let cut = cut_of(&ps, &[]);
            assert_whole(&cut, ps.len());
            let column = |line: u32| line % 2;
            // The place of a mark is unsure; it goes with the line of its row.
            for p in ps.iter().filter(|p| p.size < 10.0) {
                assert_eq!(sides.place(p), Place::Unsure, "{marks:?}");
            }
            let mut ids: Vec<usize> = (0..ps.len()).collect();
            ids.sort_by_key(|&i| ps[i].line);
            let (goes, between, ..) = rows_whole(&ps, &ids, &sides);
            assert_eq!(between, 0, "{marks:?}");
            for (&i, go) in ids.iter().zip(goes) {
                let want = if column(ps[i].line) == 0 {
                    Goes::Left
                } else {
                    Goes::Right
                };
                assert_eq!(go, want, "{marks:?} piece {i}");
            }
            let sides_of = |k: u32| -> Vec<usize> {
                (0..ps.len()).filter(|&i| column(ps[i].line) == k).collect()
            };
            assert_eq!(cut.groups, vec![sides_of(0), sides_of(1)], "{marks:?}");
        }
        // Marks at three places in a wider white space (250 to 280): the one 4 points
        // after the left column, with a gutter's width free before the right one, is a
        // piece of the left column by proof; the one in the middle is unsure and stays
        // with the line of its row; the one 4 points before the right column is a piece
        // of the right column, read there at the height of its row. No mark parts a
        // band or counts against the gutter.
        let ps = columns_with_marks(280.0, &[254.0, 262.0, 270.0]);
        let (sides, _) = gutter_of(&ps);
        let marks: Vec<usize> = (0..ps.len()).filter(|&i| ps[i].size < 10.0).collect();
        let mut ids: Vec<usize> = (0..ps.len()).collect();
        ids.sort_by_key(|&i| ps[i].line);
        let stand = places(&ps, &ids, &sides);
        let stands: Vec<Place> = marks
            .iter()
            .map(|&i| stand[ids.iter().position(|&k| k == i).unwrap()])
            .collect();
        assert_eq!(stands, [Place::Left, Place::Unsure, Place::Right]);
        let (goes, between, ..) = rows_whole(&ps, &ids, &sides);
        assert_eq!(between, 0);
        let went = |i: usize| goes[ids.iter().position(|&k| k == i).unwrap()];
        assert_eq!(
            [went(marks[0]), went(marks[1]), went(marks[2])],
            [Goes::Left, Goes::Left, Goes::Right]
        );
        let cut = cut_of(&ps, &[]);
        assert_whole(&cut, ps.len());
        assert_eq!(cut.groups.len(), 2);
        assert!(cut.groups[1].contains(&marks[2]) && !cut.groups[1].contains(&marks[1]));
        // The other way: a piece alone on its row in the white space under the last rows
        // of both columns is on the row of neither, and neither runs on through it: a row
        // of its own after them (see `a_lone_piece_in_the_gutter_reads_after_both_columns`
        // for a piece between two rows of the columns).
        let mut ps = columns_with_marks(270.0, &[]);
        ps.push(text(252.0, 268.0, 40, 590.0, 10.0, 4));
        let (sides, _) = gutter_of(&ps);
        let ids: Vec<usize> = (0..ps.len()).collect();
        assert_eq!(sides.place(&ps[ps.len() - 1]), Place::Unsure);
        assert_eq!(rows_whole(&ps, &ids, &sides).0[ps.len() - 1], Goes::Row);
    }

    #[test]
    fn a_drawn_box_from_ink_to_ink_stands_across() {
        // The cell of a header bar, a drawn box from where the ink of the left side ends
        // (310) to where the ink of the right side starts (480), is drawn across the white
        // space, whatever the title in it; within the slack of the edges.
        let cell = |x0: f32, x1: f32, boxed: Option<usize>| Piece {
            boxed,
            ..text(x0, x1, 20, 590.0, 9.0, 8)
        };
        assert_eq!(
            stands_in(two_blocks(), cell(300.0, 482.0, Some(0)), 310.0),
            Place::Across
        );
        assert_eq!(
            stands_in(two_blocks(), cell(310.5, 479.5, Some(0)), 310.0),
            Place::Across
        );
        // A box that ends before the ink of the right side is a box of the left side: it
        // reaches into the long row of that side and leaves a gutter free. Text of the
        // extent of the first box that is no drawn box is drawn across nothing: with ink
        // in the rows of both sides, into most rows of the right one only, its place is
        // unsure.
        assert_eq!(
            stands_in(two_blocks(), cell(300.0, 470.0, Some(0)), 310.0),
            Place::Left
        );
        assert_eq!(
            stands_in(two_blocks(), cell(300.0, 482.0, None), 310.0),
            Place::Unsure
        );
    }

    /// Where the last of the pieces `ps` goes at the gutter from `g0` to `g1` (see
    /// [`rows_whole`]).
    fn last_goes(ps: &[Piece], (g0, g1): (f32, f32)) -> Goes {
        let mut ids: Vec<usize> = (0..ps.len()).collect();
        ids.sort_by_key(|&i| ps[i].line);
        let sides = Sides::new(ps, &ids, (g0, g1), GUTTER_SLACK * 9.0);
        let (goes, ..) = rows_whole(ps, &ids, &sides);
        let at = ids.iter().position(|&i| i == ps.len() - 1).unwrap();
        goes[at]
    }

    #[test]
    fn an_unsure_piece_stays_with_its_row() {
        // A word from 330 to 470 at a gutter from 310 (the left side ends at 250, one row
        // at 310; the right side starts at 480) lies in the white space: its place is
        // unsure. It is given no side by a cue of its own. Its row stays whole: with a
        // word left of it on its row it goes to the left with that word, with one right of
        // it to the right, and with ink on both sides the row is read as one line between
        // the bands of the sides. Alone on its row under the last rows of both sides it is
        // a row of its own (no side runs on through it).
        let word = |x0: f32, x1: f32| Piece {
            ..text(x0, x1, 20, 590.0, 9.0, 9)
        };
        let with = |beside: &[(f32, f32)]| {
            let mut ps = two_blocks();
            ps.extend(
                beside
                    .iter()
                    .map(|&(x0, x1)| text(x0, x1, 20, 590.0, 9.0, 4)),
            );
            ps.push(word(330.0, 470.0));
            last_goes(&ps, (310.0, 480.0))
        };
        assert_eq!(with(&[]), Goes::Row);
        assert_eq!(with(&[(200.0, 240.0)]), Goes::Left);
        assert_eq!(with(&[(500.0, 540.0)]), Goes::Right);
        assert_eq!(with(&[(200.0, 240.0), (500.0, 540.0)]), Goes::Row);
        // The path of a public form: an amount set flush right in its box, mostly left of
        // a narrow gutter and a little over the start of the rows right of it, with its
        // currency sign far left on its row: it stays on the side of the sign. Its ink is
        // in the rows of the right side, so it is a line of its own there.
        let mut ps = two_blocks_to(449.0);
        for p in ps.iter_mut().filter(|p| p.line % 2 == 1) {
            p.bounds.x0 = 455.7;
        }
        let amount = Piece {
            ..text(439.6, 458.0, 20, 590.0, 7.4, 5)
        };
        let mut left = ps.clone();
        left.extend([text(276.8, 281.9, 20, 590.0, 9.0, 1), amount.clone()]);
        let ids: Vec<usize> = (0..left.len()).collect();
        let sides = Sides::new(&left, &ids, (449.5, 455.2), GUTTER_SLACK * 9.0);
        assert_eq!(sides.place(&amount), Place::Unsure);
        assert_eq!(sides.support(&amount), (1, 8));
        assert_eq!(last_goes(&left, (449.5, 455.2)), Goes::Left);
        assert_eq!(rows_whole(&left, &ids, &sides).2, [left.len() - 1]);
        // The other way: the amount set a little further left ends in the white space,
        // before the rows of the right side start. It is text of the line of its sign.
        let short = Piece {
            ..text(436.0, 454.4, 20, 590.0, 7.4, 5)
        };
        let n = left.len();
        left[n - 1] = short.clone();
        let sides = Sides::new(&left, &ids, (449.5, 455.2), GUTTER_SLACK * 9.0);
        assert_eq!(sides.place(&short), Place::Unsure);
        assert_eq!(sides.support(&short), (1, 0));
        assert_eq!(last_goes(&left, (449.5, 455.2)), Goes::Left);
        assert!(rows_whole(&left, &ids, &sides).2.is_empty());
        // With the sign right of it, it goes to the right.
        ps.extend([text(470.0, 475.0, 20, 590.0, 9.0, 1), short]);
        assert_eq!(last_goes(&ps, (449.5, 455.2)), Goes::Right);
    }

    /// The head of a statement over two blocks side by side: a title at the left (30 to
    /// 150) and, on its row, its period from 200 with ink to `ink` and blanks up to 440; a
    /// date from 440 when `date` is set; under them eight rows of a left block (to 190,
    /// three of them to 250) and eight of a right block (from 300) on baselines of their
    /// own.
    fn title_and_period(ink: f32, date: bool) -> Vec<Piece> {
        let mut ps = vec![
            text(30.0, 150.0, 0, 730.0, 10.0, 20),
            Piece {
                trail: 440.0 - ink,
                ..text(200.0, 440.0, 0, 730.0, 10.0, 16)
            },
        ];
        if date {
            ps.push(text(440.0, 500.0, 0, 730.0, 10.0, 10));
        }
        for k in 0..8u32 {
            let y = 700.0 - 12.0 * k as f32;
            let end = if k % 3 == 1 { 250.0 } else { 190.0 };
            ps.push(text(30.0, end, 2 * k + 1, y, 10.0, 40));
            ps.push(text(300.0, 565.0, 2 * k + 2, y - 5.0, 10.0, 40));
        }
        ps
    }

    #[test]
    fn a_title_keeps_the_period_on_its_row() {
        // The private finding P1 of Task 31b in small. The period starts before the gutter
        // (250 to 300) and its ink ends at 320, after the start of the right block: it
        // reaches into all rows of the right side and into the three long rows of the left
        // one, with an edge of none. With the title left of it and the date right of it
        // no side takes the row whole: it is read at its height, title, period, date,
        // before both blocks, each of the three a group of its own.
        let ps = title_and_period(320.0, true);
        let (sides, why) = gutter_of(&ps);
        assert_eq!(why, Why::Prose);
        assert_eq!(sides.place(&ps[1]), Place::Unsure);
        let cut = cut_of(&ps, &[]);
        assert_whole(&cut, ps.len());
        let block = |from: usize| (0..8).map(|k| from + 2 * k).collect::<Vec<usize>>();
        assert_eq!(
            cut.groups,
            vec![vec![0], vec![1], vec![2], block(3), block(4)]
        );
        assert!(cut.sides.is_empty() && cut.breaks.is_empty());
        // Without the date the ink of the row beside the period is the title, on the left:
        // the period goes with it, and the left block reads on under them. The period has
        // ink in the rows of the right side, so it is a line of its own.
        let ps = title_and_period(320.0, false);
        let cut = cut_of(&ps, &[]);
        assert_eq!(cut.groups.concat()[..3], [0, 1, 2], "{:?}", cut.groups);
        assert_eq!((cut.sides.len(), &cut.breaks[..]), (0, &[1][..]));
        // A period that ends in the white space (at 280) and leaves a gutter free is a
        // piece of the left side, and text of the line of its title.
        let ps = title_and_period(280.0, false);
        let (sides, _) = gutter_of(&ps);
        assert_eq!(sides.place(&ps[1]), Place::Left);
        let cut = cut_of(&ps, &[]);
        assert_eq!(cut.groups.concat()[..3], [0, 1, 2], "{:?}", cut.groups);
        assert!(cut.sides.is_empty() && cut.breaks.is_empty());
        // The blanks after the period are no ink: a period whose ink ends before the
        // gutter (at 240) is a piece of the left side, whatever its blanks run up to, and
        // the date a piece of the right side. Both blocks read whole.
        let ps = title_and_period(240.0, true);
        let (sides, _) = gutter_of(&ps);
        assert!(!sides.reaches(&ps[1]), "{sides:?}");
        assert_eq!(sides.place(&ps[1]), Place::Left);
        // The other way: with the blanks taken for ink the period would reach into the
        // gutter and into the rows of the right side.
        let mut inked = ps.clone();
        inked[1].trail = 0.0;
        let (sides, _) = gutter_of(&inked);
        assert!(sides.reaches(&inked[1]), "{sides:?}");
        assert_eq!(sides.place(&inked[1]), Place::Unsure);
    }

    /// Two columns of prose of ten rows at 10 points and a pitch of 12 (30 to 250 and 280
    /// to 565), the right one `shift` lower. The rows from row 5 on of the left and of
    /// the right one lie `drop` lower still: white space under row 4. With no shift and
    /// one drop the two columns share their rows.
    fn prose_columns(shift: f32, drop: (f32, f32)) -> Vec<Piece> {
        let mut ps = Vec::new();
        for k in 0..10u32 {
            let y = 700.0 - 12.0 * k as f32;
            let low = |d: f32| if k > 4 { d } else { 0.0 };
            let right = if shift == 0.0 && drop.0 == drop.1 {
                2 * k
            } else {
                2 * k + 1
            };
            ps.push(text(30.0, 250.0, 2 * k, y - low(drop.0), 10.0, 40));
            ps.push(text(280.0, 565.0, right, y - shift - low(drop.1), 10.0, 40));
        }
        ps
    }

    /// The pieces `ps` with one more, `q`, sorted top down and their rows numbered by
    /// their baselines (pieces on one baseline share a row): the place of `q`, the pieces
    /// of the left and of the right column of [`prose_columns`] top down, and all pieces.
    fn with_lone(mut ps: Vec<Piece>, q: Piece) -> (usize, Vec<usize>, Vec<usize>, Vec<Piece>) {
        let chars = q.chars;
        ps.push(q);
        ps.sort_by(|a, b| {
            b.baseline
                .total_cmp(&a.baseline)
                .then(a.bounds.x0.total_cmp(&b.bounds.x0))
        });
        let (mut line, mut last) = (0, f32::NAN);
        for p in &mut ps {
            line += u32::from(p.baseline != last);
            last = p.baseline;
            p.line = line;
        }
        let at = ps.iter().position(|p| p.chars == chars).unwrap();
        let of =
            |x: f32| -> Vec<usize> { (0..ps.len()).filter(|&i| ps[i].bounds.x0 == x).collect() };
        (at, of(30.0), of(280.0), ps)
    }

    #[test]
    fn a_lone_piece_in_the_gutter_reads_after_both_columns() {
        // Review Q1 of Task 31b and its corrected ruling. Two columns of prose of ten rows
        // at 10 points (30 to 250 and 280 to 565), the right one on baselines of its own or
        // on those of the left one, and one piece on a baseline of its own in the white
        // space between rows 4 and 5: a mark of 6 points or a short word of 10. It is the
        // only ink of its row, and both columns run on through its height at their pitch
        // of 12: both columns read whole, the piece after them, and it does not count
        // against the gutter.
        for (shift, size, (x0, x1)) in [
            (5.0, 6.0, (256.0, 274.0)),
            (0.0, 6.0, (256.0, 274.0)),
            (5.0, 10.0, (256.0, 274.0)),
        ] {
            let mut ps = prose_columns(shift, (0.0, 0.0));
            ps.push(text(x0, x1, 40, 646.0, size, 3));
            let (sides, _) = gutter_of(&ps);
            assert_eq!(sides.place(&ps[20]), Place::Unsure, "{shift} {size} {x0}");
            assert_eq!(sides.near(&ps[20]), None, "{shift} {size} {x0}");
            let mut ids: Vec<usize> = (0..ps.len()).collect();
            ids.sort_by_key(|&i| ps[i].line);
            assert_eq!(rows_whole(&ps, &ids, &sides).1, 0);
            let cut = cut_of(&ps, &[]);
            assert_whole(&cut, ps.len());
            let column = |from: usize| (0..10).map(|k| from + 2 * k).collect::<Vec<usize>>();
            assert_eq!(
                cut.groups,
                vec![column(0), column(1), vec![20]],
                "{shift} {size} {x0}"
            );
        }
        // The pitch moved both ways (`LONE_PITCH`, 15 % of the pitch of 12): with the rows
        // under the piece 1.2 points lower the columns still run on through it, and it
        // reads after them. With 3.6 points of white space more on both sides, both
        // columns break their pitch at the piece: the piece is a row of its own between
        // the bands, after rows 0 to 4 of both columns and before rows 5 to 9 (a heading
        // of what follows it), and it counts against the gutter.
        //
        // Design ruling of fix round 8 (review R2 of Task 31b): with the white space on
        // one side only, that side breaks its pitch and the other runs on, most likely
        // through a sentence. The running column is not parted: it reads whole. The piece
        // goes with the side that breaks, alone between rows 0 to 4 and rows 5 to 9 of
        // that side, and it parts no band and counts against no gutter.
        for (x0, x1) in [(256.0, 274.0), (258.0, 272.0)] {
            for (drop, want) in [
                ((1.2, 1.2), Goes::After),
                ((1.2, 0.0), Goes::After),
                ((0.0, 1.2), Goes::After),
                ((3.6, 3.6), Goes::Row),
                ((3.6, 0.0), Goes::HeadLeft),
                ((0.0, 3.6), Goes::HeadRight),
                ((3.6, 1.2), Goes::HeadLeft),
            ] {
                let (at, left, right, ps) =
                    with_lone(prose_columns(5.0, drop), text(x0, x1, 40, 646.0, 10.0, 3));
                let (sides, _) = gutter_of(&ps);
                let ids: Vec<usize> = (0..ps.len()).collect();
                let (goes, between, breaks, _) = rows_whole(&ps, &ids, &sides);
                assert_eq!(goes[at], want, "{x0} {drop:?}");
                assert_eq!(between, usize::from(want == Goes::Row), "{x0} {drop:?}");
                assert!(breaks.is_empty());
                let cut = cut_of(&ps, &[]);
                assert_whole(&cut, ps.len());
                let (top, low) = (
                    |side: &[usize]| side[..5].to_vec(),
                    |side: &[usize]| side[5..].to_vec(),
                );
                let groups = match want {
                    Goes::After => vec![left, right, vec![at]],
                    Goes::Row => vec![top(&left), top(&right), vec![at], low(&left), low(&right)],
                    Goes::HeadLeft => vec![top(&left), vec![at], low(&left), right],
                    _ => vec![left, top(&right), vec![at], low(&right)],
                };
                assert_eq!(cut.groups, groups, "{x0} {drop:?}");
                // The piece that heads a part of a side stands between two bands of that
                // side; the gutter parts the sides as it does without the piece.
                let splits = match want {
                    Goes::HeadLeft => vec![Split::Band, Split::Band, Split::Gutter],
                    Goes::HeadRight => vec![Split::Gutter, Split::Band, Split::Band],
                    Goes::After => vec![Split::Gutter; 2],
                    _ => vec![Split::Gutter; 4],
                };
                assert_eq!(cut.splits, splits, "{x0} {drop:?}");
            }
        }
    }

    #[test]
    fn a_lone_piece_near_one_column_is_no_line_of_it() {
        // Review R1 of Task 31b. A short word 2 points after the ragged end of the left
        // column and 18 points before the right one, alone on its row between rows 4 and
        // 5 of the left column. A mark on row 1 of the left column, 6 points before the
        // right column, crosses the rest of the white space, so the gutter is the whole
        // white space (250 to 280) and the word reaches into it. It stands near the left
        // side, and beside other ink of its row that proves the side. Alone on its row
        // it proves nothing: the ink of the sides is one row each and says nothing of a
        // row of its own. Both columns run on through the word at their pitch, so it
        // parts nothing and reads after both columns, inside no sentence of the left one.
        let page = |drop: (f32, f32), beside: bool| {
            let mut ps = prose_columns(5.0, drop);
            ps.push(text(266.0, 274.0, 0, 688.0, 6.0, 2));
            if beside {
                ps.push(text(100.0, 200.0, 0, 646.0, 10.0, 12));
            }
            with_lone(ps, text(252.0, 262.0, 0, 646.0, 10.0, 3))
        };
        let (at, left, right, ps) = page((0.0, 0.0), false);
        let mark = ps.iter().position(|p| p.chars == 2).unwrap();
        let (sides, _) = gutter_of(&ps);
        assert!(sides.reaches(&ps[at]));
        assert_eq!(sides.place(&ps[at]), Place::Unsure);
        assert_eq!(sides.near(&ps[at]), Some(Place::Left));
        let ids: Vec<usize> = (0..ps.len()).collect();
        let stands = places(&ps, &ids, &sides);
        // The mark has the line of the left column on its row: a piece of the right
        // column, which it stands near. The word has no ink beside it.
        assert_eq!((stands[mark], stands[at]), (Place::Right, Place::Unsure));
        let (goes, between, ..) = rows_whole(&ps, &ids, &sides);
        assert_eq!((goes[at], between), (Goes::After, 0));
        let cut = cut_of(&ps, &[]);
        assert_whole(&cut, ps.len());
        assert_eq!(cut.groups.first(), Some(&left));
        assert_eq!(cut.groups.last(), Some(&vec![at]));
        let between: Vec<usize> = cut.groups[1..cut.groups.len() - 1].concat();
        let mut rest = right.clone();
        rest.push(mark);
        rest.sort_unstable();
        assert_eq!(between, rest);
        // Where the left column breaks its pitch at the word, it heads the part of that
        // column under it; the right column reads whole.
        let (at, left, _, ps) = page((3.6, 0.0), false);
        let (sides, _) = gutter_of(&ps);
        assert_eq!(rows_whole(&ps, &ids, &sides).0[at], Goes::HeadLeft);
        let cut = cut_of(&ps, &[]);
        assert_eq!(
            cut.groups[..3],
            [left[..5].to_vec(), vec![at], left[5..].to_vec()]
        );
        // The other way: with a short line of the left column on its row, the word is a
        // piece of that column by the white space it leaves (`near`), text of that line.
        let (at, _, _, ps) = page((0.0, 0.0), true);
        let line = ps.iter().position(|p| p.chars == 12).unwrap();
        let (sides, _) = gutter_of(&ps);
        let ids: Vec<usize> = (0..ps.len()).collect();
        assert_eq!(places(&ps, &ids, &sides)[at], Place::Left);
        assert_eq!(rows_whole(&ps, &ids, &sides).0[at], Goes::Left);
        let cut = cut_of(&ps, &[]);
        let group = cut.groups.iter().find(|g| g.contains(&at)).unwrap();
        assert!(group.contains(&line) && group.contains(&left_of(&ps)[0]));
    }

    /// The pieces of the left column of [`prose_columns`] among `ps`.
    fn left_of(ps: &[Piece]) -> Vec<usize> {
        (0..ps.len()).filter(|&i| ps[i].bounds.x0 == 30.0).collect()
    }

    #[test]
    fn a_lone_piece_beside_a_side_that_says_nothing_is_a_row_of_its_own() {
        // The other way of the design ruling of fix round 8 (review R2 of Task 31b):
        // which side breaks must be shown. The left column runs on through the piece at
        // its pitch. The right column says nothing at its height: it ends over the piece
        // (rows 0 to 4 only), or starts under it (rows 5 to 9 only), or it has two rows,
        // one over and one under the piece, and so no pitch that could break. The piece
        // is a row of its own between the bands, as before the ruling.
        let page = |keep: &dyn Fn(usize) -> bool| {
            let ps: Vec<Piece> = prose_columns(5.0, (0.0, 0.0))
                .into_iter()
                .enumerate()
                .filter(|(i, _)| i % 2 == 0 || keep(i / 2))
                .map(|(_, p)| p)
                .collect();
            with_lone(ps, text(256.0, 274.0, 40, 646.0, 10.0, 3))
        };
        for (name, keep) in [
            ("ends", &(|k: usize| k < 5) as &dyn Fn(usize) -> bool),
            ("starts", &|k: usize| k > 4),
            ("two rows", &|k: usize| k == 4 || k == 5),
        ] {
            let (at, left, right, ps) = page(keep);
            let (sides, _) = gutter_of(&ps);
            let ids: Vec<usize> = (0..ps.len()).collect();
            let (goes, between, ..) = rows_whole(&ps, &ids, &sides);
            assert_eq!((goes[at], between), (Goes::Row, 1), "{name}");
            let cut = cut_of(&ps, &[]);
            assert_whole(&cut, ps.len());
            let k = cut.groups.iter().position(|g| g == &vec![at]);
            let before: Vec<usize> = cut.groups[..k.unwrap()].concat();
            let over = |side: &[usize]| side.iter().filter(|&&i| i < at).count();
            assert_eq!(before.len(), over(&left) + over(&right), "{name}");
            assert!(before.iter().all(|&i| i < at), "{name}");
        }
        // Rows whose baselines are one are one row of the side (traced on a public page,
        // arxiv-2408.09869 page 7: marks set a hair off the baselines of a column made
        // its pitch a fraction of a point, and every line a break of it). With a mark on a
        // row of its own 0.5 points over each line of the right column (within
        // `BASELINE_NEAR`, 1.5 points at this size), the pitch of the column is 12 still:
        // it runs on through the piece. Marks 2 points over the lines are rows of their
        // own: the steps are 2 and 10, which is no pitch of lines (see `LEAST_PITCH`),
        // so the column says nothing and the piece is a row of its own.
        for (off, want) in [(0.5, Goes::After), (2.0, Goes::Row)] {
            let mut ps = prose_columns(5.0, (0.0, 0.0));
            for k in 0..10 {
                let y = 700.0 - 12.0 * k as f32 - 5.0 + off;
                ps.push(text(560.0, 565.0, 0, y, 10.0, 1));
            }
            let (at, .., ps) = with_lone(ps, text(256.0, 274.0, 40, 646.0, 10.0, 3));
            let (sides, _) = gutter_of(&ps);
            let ids: Vec<usize> = (0..ps.len()).collect();
            assert_eq!(rows_whole(&ps, &ids, &sides).0[at], want, "{off}");
        }
        // A side whose rows stand closer than their letters allow is no column of lines
        // (`LEAST_PITCH`, 0.8 sizes: 8 points at this size), moved both ways. The right
        // side at a pitch of 7.9 has no pitch that could run on or break: the piece is a
        // row of its own. At a pitch of 8.1 it is a column that runs on through the
        // piece, as the left one does: the piece reads after both.
        for (pitch, want) in [(7.9, Goes::Row), (8.1, Goes::After)] {
            let mut ps: Vec<Piece> = prose_columns(5.0, (0.0, 0.0))
                .into_iter()
                .step_by(2)
                .collect();
            for k in 0..14 {
                ps.push(text(280.0, 565.0, 0, 695.0 - pitch * k as f32, 10.0, 40));
            }
            let (at, .., ps) = with_lone(ps, text(256.0, 274.0, 40, 646.0, 10.0, 3));
            let (sides, _) = gutter_of(&ps);
            let ids: Vec<usize> = (0..ps.len()).collect();
            assert_eq!(rows_whole(&ps, &ids, &sides).0[at], want, "{pitch}");
        }
        // With a third row at the pitch the right column has a pitch again, and it runs
        // on through the piece: both columns read whole, the piece after them.
        let (at, left, right, ps) = page(&|k| (3..=5).contains(&k));
        let (sides, _) = gutter_of(&ps);
        let ids: Vec<usize> = (0..ps.len()).collect();
        assert_eq!(rows_whole(&ps, &ids, &sides).0[at], Goes::After);
        assert_eq!(cut_of(&ps, &[]).groups, vec![left, right, vec![at]]);
    }

    #[test]
    fn a_short_centred_heading_reads_before_both_columns_under_it() {
        // Two columns of prose, in two bands, with a short heading between the bands that is
        // centred over the gutter (160..180): inside it, or a little wider than it. Inside
        // it its place is unsure and it is the only ink of its row, but both columns break
        // their pitch at it (70 points from the row over it to the row under it, against
        // 12): it is a row of its own between the bands. Wider, it stands over the rows of
        // both sides, proven across. Both ways it reads after both columns over it and
        // before both columns under it.
        let columns = |ps: &mut Vec<Piece>, top: f32, line: u32| {
            for k in 0..6u32 {
                let y = top - 12.0 * k as f32;
                ps.push(text(50.0, 160.0, line + k, y, 10.0, 24));
                ps.push(text(180.0, 290.0, line + k, y, 10.0, 25));
            }
        };
        for (x0, x1) in [(164.0, 176.0), (150.0, 190.0)] {
            let mut ps = Vec::new();
            columns(&mut ps, 700.0, 0);
            ps.push(text(x0, x1, 6, 600.0, 10.0, 4));
            columns(&mut ps, 570.0, 7);
            let cut = cut_of(&ps, &[]);
            assert_whole(&cut, ps.len());
            let side = |from: usize| (0..6).map(|k| from + 2 * k).collect::<Vec<usize>>();
            assert_eq!(
                cut.groups,
                vec![side(0), side(1), vec![12], side(13), side(14)],
                "{x0} {x1}"
            );
        }
        // The other way: a short line from inside the gutter into the right column is a line
        // of that column. The left column reads whole, over and under it.
        let mut ps = Vec::new();
        columns(&mut ps, 700.0, 0);
        ps.push(text(172.0, 230.0, 6, 600.0, 10.0, 12));
        columns(&mut ps, 570.0, 7);
        let cut = cut_of(&ps, &[]);
        assert_whole(&cut, ps.len());
        let left: Vec<usize> = (0..6)
            .map(|k| 2 * k)
            .chain((0..6).map(|k| 13 + 2 * k))
            .collect();
        assert_eq!(cut.groups.concat()[..12], left[..], "{:?}", cut.groups);
    }

    #[test]
    fn a_piece_over_the_whole_width_of_a_side_does_not_go_to_the_other() {
        // The path of a public form (irs-f6744 untagged page 174). A block of five short
        // lines at the left (from 40) and a block of rows at the right (from 112), on
        // baselines of their own. Over them one row: three labels set as one piece from 40
        // to 203, over the left block and into two of the four rows of the right one (no
        // place is proven), and a value at 218 on its row, a piece of the right side. The
        // piece runs the whole width of the left side, so it is no line of the right one:
        // its row is kept whole and reads first, the piece a line of its own.
        let page = |from: f32| {
            let mut ps = vec![
                text(from, 203.5, 0, 390.7, 5.4, 60),
                text(218.6, 220.6, 0, 390.7, 5.4, 1),
            ];
            // The line under the row is set in a little: the row shares no edge with it.
            let left = [
                (45.0, 384.6),
                (47.9, 373.7),
                (40.3, 361.5),
                (40.3, 352.2),
                (40.3, 343.0),
            ];
            for (k, (x0, y)) in left.into_iter().enumerate() {
                ps.push(text(x0, 90.0, 2 * k as u32 + 1, y, 5.4, 16));
            }
            for (k, x0) in [112.4, 112.4, 205.0].into_iter().enumerate() {
                let y = 379.9 - 9.2 * k as f32;
                ps.push(text(x0, 222.0, 2 * k as u32 + 2, y, 5.4, 14));
            }
            ps
        };
        let ps = page(40.3);
        let (sides, why) = gutter_of(&ps);
        assert_eq!(why, Why::Offset);
        assert_eq!(sides.place(&ps[0]), Place::Unsure);
        assert_eq!(sides.spans(&ps[0]), (true, false));
        let ids: Vec<usize> = (0..ps.len()).collect();
        let mut by_line = ids.clone();
        by_line.sort_by_key(|&i| ps[i].line);
        let (goes, between, breaks, _) = rows_whole(&ps, &by_line, &sides);
        assert_eq!((goes[0], goes[1], between), (Goes::Row, Goes::Row, 1));
        assert_eq!(breaks, vec![0]);
        let cut = cut_of(&ps, &[]);
        assert_whole(&cut, ps.len());
        assert_eq!(cut.groups[..2], [vec![0], vec![1]]);
        assert_eq!(cut.breaks, vec![0]);
        // The other way: the same piece from 60, inside the left block (most of its rows
        // start before it), runs the width of no side. Its place is as unsure, and it goes
        // with the ink of its row, to the right, as a line of its own there.
        let ps = page(60.0);
        let (sides, _) = gutter_of(&ps);
        assert_eq!(sides.place(&ps[0]), Place::Unsure);
        assert_eq!(sides.spans(&ps[0]), (false, false));
        let (goes, between, breaks, _) = rows_whole(&ps, &by_line, &sides);
        assert_eq!((goes[0], goes[1], between), (Goes::Right, Goes::Right, 0));
        assert_eq!(breaks, vec![0]);
        // The mirror (review R5 of Task 31b): the same page turned left to right. The
        // row of labels runs from inside the left block to the outer edge of the right
        // one, where most of the rows of the right block end, with the value at the left
        // of its row, a piece of the left side: the piece runs the whole width of the
        // right side and is no line of the left one. Its row is kept whole.
        let turned = |from: f32| -> Vec<Piece> {
            let mut ps = page(from);
            for p in &mut ps {
                (p.bounds.x0, p.bounds.x1) = (262.3 - p.bounds.x1, 262.3 - p.bounds.x0);
            }
            ps
        };
        let ps = turned(40.3);
        let (sides, why) = gutter_of(&ps);
        assert_eq!(why, Why::Offset);
        assert_eq!(sides.place(&ps[0]), Place::Unsure);
        assert_eq!(sides.spans(&ps[0]), (false, true));
        let (goes, between, breaks, _) = rows_whole(&ps, &by_line, &sides);
        assert_eq!((goes[0], goes[1], between), (Goes::Row, Goes::Row, 1));
        assert_eq!(breaks, vec![0]);
        let cut = cut_of(&ps, &[]);
        assert_whole(&cut, ps.len());
        assert_eq!(cut.groups[..2], [vec![1], vec![0]]);
        // The other way: the piece ends inside the right block (most of its rows end
        // after it). It goes with the ink of its row, to the left.
        let ps = turned(60.0);
        let (sides, _) = gutter_of(&ps);
        assert_eq!(sides.spans(&ps[0]), (false, false));
        let (goes, between, breaks, _) = rows_whole(&ps, &by_line, &sides);
        assert_eq!((goes[0], goes[1], between), (Goes::Left, Goes::Left, 0));
        assert_eq!(breaks, vec![0]);
    }

    #[test]
    fn the_rows_a_piece_spans_are_counted_by_a_search() {
        // Review R5 of Task 31b: a form of many rows, each a label from the left edge of
        // the left block over the white space into the right block (into the ink of every
        // fourth line of that block: no place is proven), with a value of the right block
        // on its row and a short line of the left block under it, set in a little. Each
        // label is asked whether it runs the whole width of the left side, and each does:
        // every row is kept whole. One question is a search in the sorted starts of the
        // rows, so twice the rows take about twice the steps, not four times.
        let steps = |rows: u32| {
            let mut ps = Vec::new();
            for k in 0..rows {
                let y = 9000.0 - 12.0 * k as f32;
                ps.push(text(40.0, 180.0, 2 * k, y, 10.0, 30));
                ps.push(text(230.0, 260.0, 2 * k, y, 10.0, 6));
                ps.push(text(45.0, 90.0, 2 * k + 1, y - 6.0, 10.0, 9));
                if k % 4 == 0 {
                    ps.push(text(170.0, 260.0, 2 * k + 1, y - 6.0, 10.0, 18));
                }
            }
            let ids: Vec<usize> = (0..ps.len()).collect();
            let sides = Sides::new(&ps, &ids, (100.0, 170.0), GUTTER_SLACK * 10.0);
            let before = work::work_steps();
            let (goes, between, ..) = rows_whole(&ps, &ids, &sides);
            assert_eq!(between, rows as usize);
            assert!(
                ids.iter()
                    .zip(&goes)
                    .all(|(&i, &go)| (go == Goes::Row) == (ps[i].line % 2 == 0))
            );
            work::work_steps().zip(before).map(|(a, b)| a - b)
        };
        if let (Some(one), Some(two)) = (steps(200), steps(400)) {
            assert!(10 * two <= 21 * one, "{one} {two}");
        }
    }

    /// A head of two blocks of six lines side by side (to 150 and from 165), a rule under
    /// them, and under the rule one row of three short labels, the middle one from
    /// `middle`. Gives the pieces, the rule and the indices of the labels.
    fn labels_under_a_head(middle: f32) -> (Vec<Piece>, Vec<Line>, [usize; 3]) {
        let mut ps = Vec::new();
        for k in 0..6u32 {
            let y = 700.0 - 12.0 * k as f32;
            ps.push(text(36.0, 150.0, k, y, 10.0, 24));
            ps.push(text(165.0, 440.0, k, y, 10.0, 60));
        }
        let n = ps.len();
        ps.push(text(82.0, 104.0, 6, 610.0, 10.0, 6));
        ps.push(text(middle, middle + 16.0, 6, 610.0, 10.0, 6));
        ps.push(text(323.0, 382.0, 6, 610.0, 10.0, 20));
        let rule = Line {
            at: 625.0,
            from: 36.0,
            to: 440.0,
        };
        (ps, vec![rule], [n, n + 1, n + 2])
    }

    #[test]
    fn a_band_whose_rows_fill_the_gutter_is_looked_at_alone() {
        // The path of a public form: a rule parts a row of labels from the head blocks over
        // it, and one of the labels stands where the head blocks have their gutter (150 to
        // 165). The row has no such gutter: it reads whole, as one group.
        let (ps, rules, labels) = labels_under_a_head(157.0);
        let (sides, _) = gutter_of(&ps);
        assert!(sides.reaches(&ps[labels[1]]), "{sides:?}");
        let cut = cut_of(&ps, &rules);
        assert_whole(&cut, ps.len());
        assert!(cut.groups.contains(&labels.to_vec()), "{:?}", cut.groups);
        // The other way: the row leaves the gutter white (its middle label starts at 170),
        // and the gutter parts it as it parts the rows over it.
        let (ps, rules, labels) = labels_under_a_head(170.0);
        let cut = cut_of(&ps, &rules);
        assert_whole(&cut, ps.len());
        assert!(cut.groups.contains(&vec![labels[0]]), "{:?}", cut.groups);
        assert!(
            cut.groups.contains(&labels[1..].to_vec()),
            "{:?}",
            cut.groups
        );
        // Most rows decide: with a second row of labels under the first that leaves the
        // gutter white, one row of two fills it, and the band is parted by the gutter.
        let (mut ps, rules, labels) = labels_under_a_head(157.0);
        ps.push(text(82.0, 104.0, 7, 598.0, 10.0, 6));
        ps.push(text(323.0, 382.0, 7, 598.0, 10.0, 20));
        let cut = cut_of(&ps, &rules);
        assert_whole(&cut, ps.len());
        assert!(
            !cut.groups
                .iter()
                .any(|g| labels.iter().all(|l| g.contains(l)))
        );
    }

    /// A ragged paragraph of fourteen lines from 30, with a block of three short lines
    /// right of it on baselines of their own. The lines of the paragraph from the fifth on
    /// end at `long`, one each; the first line of the block starts at `first`.
    fn paragraph_beside_a_block(long: &[f32], first: f32) -> Vec<Piece> {
        let ends = [372.0, 351.0, 380.0, 344.0, 366.0, 358.0, 377.0];
        let mut ps = Vec::new();
        for k in 0..14usize {
            let end = long
                .get(k.wrapping_sub(4))
                .copied()
                .unwrap_or(ends[k % ends.len()]);
            ps.push(text(
                30.0,
                end,
                2 * k as u32,
                700.0 - 12.0 * k as f32,
                10.0,
                60,
            ));
        }
        for k in 0..3u32 {
            ps.push(text(
                if k == 0 { first } else { 440.0 },
                560.0,
                2 * k + 1,
                694.0 - 12.0 * k as f32,
                10.0,
                20,
            ));
        }
        ps
    }

    #[test]
    fn lines_that_reach_into_a_gutter_do_not_count_against_it() {
        // Reviews N10 and P5 of Task 31b. Two lines of the paragraph end in the white space
        // before the block (at 437 and 431) and the first line of the block starts in it
        // (at 433): three pieces reach into the gutter, which is found from the ends of
        // the other lines (380) to the block (440), and two pieces of seventeen may cross
        // one. They only reach into it, each from its side: the gutter stands, and the
        // paragraph reads whole before the block.
        let ps = paragraph_beside_a_block(&[437.0, 431.0], 433.0);
        let (sides, _) = gutter_of(&ps);
        assert!(sides.g0 < 385.0 && sides.g1 > 435.0, "{sides:?}");
        let reaching: Vec<Place> = ps
            .iter()
            .filter(|p| sides.reaches(p))
            .map(|p| sides.place(p))
            .collect();
        assert_eq!(reaching, [Place::Left, Place::Left, Place::Right]);
        let cut = cut_of(&ps, &[]);
        assert_whole(&cut, ps.len());
        let order = cut.groups.concat();
        assert_eq!(order[..14], (0..14).collect::<Vec<usize>>()[..]);
        // The other way: three lines over both sides (to the right edge of the block) cover
        // the white space, more than may: no band is left for a gutter.
        let ps = paragraph_beside_a_block(&[560.0, 560.0, 560.0], 440.0);
        let ids: Vec<usize> = (0..ps.len()).collect();
        assert!(gutter(&ps, &ids).is_none());
        // Two such lines are within the share: they part the bands of the gutter.
        let ps = paragraph_beside_a_block(&[560.0, 560.0], 440.0);
        let (sides, _) = gutter_of(&ps);
        assert_eq!(sides.place(&ps[4]), Place::Across);
    }

    #[test]
    fn pieces_across_count_against_a_gutter() {
        // Review P5 of Task 31b: a shape where the count of the pieces across is the reason.
        // Two blocks of six lines on baselines of their own, a line across both under them,
        // a shorter line under it from its left edge (the last line of its paragraph) and
        // one that ends at its right edge. The short lines leave four points of the white
        // space free between them, too little for a band: the band of the gutter is all of
        // the white space, no step of it under more than two pieces. But three pieces stand
        // across, of fifteen: more than the share, no gutter.
        let page = |right_flush: bool| {
            let mut ps = Vec::new();
            for k in 0..6u32 {
                let y = 700.0 - 12.0 * k as f32;
                ps.push(text(30.0, 200.0, 2 * k, y, 10.0, 30));
                ps.push(text(300.0, 470.0, 2 * k + 1, y - 5.0, 10.0, 30));
            }
            ps.push(text(30.0, 470.0, 20, 600.0, 10.0, 80));
            ps.push(text(30.0, 248.0, 21, 588.0, 10.0, 40));
            if right_flush {
                ps.push(text(252.0, 470.0, 22, 576.0, 10.0, 40));
            }
            ps
        };
        let ps = page(true);
        let ids: Vec<usize> = (0..ps.len()).collect();
        let band = best_band(&ps, &ids, 2, 10.0).expect("a band");
        assert!(band.0 < 205.0 && band.1 > 295.0, "{band:?}");
        let sides = Sides::new(&ps, &ids, band, GUTTER_SLACK * 10.0);
        let across = ps.iter().filter(|p| sides.place(p) == Place::Across);
        assert_eq!(across.count(), 3);
        assert!(gutter(&ps, &ids).is_none());
        // The other way: without the third piece across the gutter stands.
        let ps = page(false);
        let ids: Vec<usize> = (0..ps.len()).collect();
        assert!(gutter(&ps, &ids).is_some());
    }

    #[test]
    fn rules_in_pieces_mark_the_rows_end_to_end() {
        // The same rows with the upper rule handed over as drawn, in pieces: `joined` finds
        // the joints and the joined rule, and the cut reads the rows whole.
        let (ps, rules, firsts) = rows_between_rules();
        let piece = |from: f32, to: f32| Line {
            at: 696.0,
            from,
            to,
        };
        let drawn = [
            piece(30.0, 57.0),
            piece(57.0, 460.0),
            piece(460.0, 518.0),
            piece(518.0, 560.0),
            rules[1],
        ];
        // The rules go to the cut as a reader hands them over: the pieces as drawn for the
        // bands (none of them wide), the joined rule and its joints for the block.
        let (rules_joined, joints) = joined(&drawn, &[]);
        assert_eq!(rules_joined.len(), 1);
        assert_eq!(joints.len(), 3);
        let ruled = Ruled {
            rules: &drawn,
            joined: &rules_joined,
            joints: &joints,
        };
        let cut = xy_cut_ruled(&ps, &ruled, &Limits::default(), &|| false);
        assert_whole(&cut, ps.len());
        let order = cut.groups.concat();
        let at = |i: usize| order.iter().position(|&o| o == i).unwrap();
        assert!(
            firsts
                .iter()
                .all(|&f| (1..4).all(|c| at(f + c) == at(f) + c)),
            "{:?}",
            cut.groups
        );
    }

    /// `copies` pages of [`rows_between_rules`], one under the other, with the upper rule of
    /// each drawn in four pieces: the pieces, the rules as drawn, and the joined rules with
    /// their joints.
    fn pages_of_rows(copies: usize) -> (Vec<Piece>, Vec<Line>) {
        let (page, rules, _) = rows_between_rules();
        let lines = page.iter().map(|p| p.line).max().unwrap_or(0) + 1;
        let (mut ps, mut drawn) = (Vec::new(), Vec::new());
        for n in 0..copies {
            let down = 400.0 * n as f32;
            ps.extend(page.iter().map(|p| Piece {
                bounds: Bounds {
                    y0: p.bounds.y0 - down,
                    y1: p.bounds.y1 - down,
                    ..p.bounds
                },
                line: p.line + lines * n as u32,
                baseline: p.baseline - down,
                ..p.clone()
            }));
            for (from, to) in [(30.0, 57.0), (57.0, 460.0), (460.0, 518.0), (518.0, 560.0)] {
                drawn.push(Line {
                    at: rules[0].at - down,
                    from,
                    to,
                });
            }
            drawn.push(Line {
                at: rules[1].at - down,
                ..rules[1]
            });
        }
        (ps, drawn)
    }

    #[test]
    fn the_work_of_ruled_blocks_grows_with_the_page() {
        // Review I4 and N9 of Task 31b: the cut of a page with rules in pieces, joints and
        // blocks between rules, measured as a whole (the joints, the rules of each region,
        // the rows and the joints over each block). Twice the page (twice the pieces, the
        // rules and the joints) takes at most twice the steps and a tenth more: the regions
        // of each level look at the rules of their own heights only.
        let steps = |copies: usize| {
            let (ps, drawn) = pages_of_rows(copies);
            let before = work::work_steps();
            let (rules_joined, joints) = joined(&drawn, &[]);
            let ruled = Ruled {
                rules: &drawn,
                joined: &rules_joined,
                joints: &joints,
            };
            let cut = xy_cut_ruled(&ps, &ruled, &Limits::default(), &|| false);
            assert_whole(&cut, ps.len());
            work::work_steps().zip(before).map(|(a, b)| a - b)
        };
        if let (Some(four), Some(eight)) = (steps(4), steps(8)) {
            assert!(10 * eight <= 22 * four, "{four} {eight}");
        }
    }

    #[test]
    fn joints_that_an_upright_line_meets_are_corners_of_a_grid() {
        let piece = |from: f32, to: f32| Line {
            at: 500.0,
            from,
            to,
        };
        let rules = [piece(30.0, 60.0), piece(60.0, 300.0), piece(300.0, 560.0)];
        let (rules_joined, joints) = joined(&rules, &[]);
        assert_eq!(joints, vec![(500.0, 60.0), (500.0, 300.0)]);
        assert_eq!(rules_joined.len(), 1);
        // An upright line down from the first joint: that joint is a corner of a drawn cell.
        let upright = Line {
            at: 60.0,
            from: 440.0,
            to: 500.0,
        };
        let (_, joints) = joined(&rules, &[upright]);
        assert_eq!(joints, vec![(500.0, 300.0)]);
        // A rule in one piece has no joint, and nothing is joined.
        let (rules_joined, joints) = joined(&rules[1..2], &[]);
        assert!(rules_joined.is_empty() && joints.is_empty());
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
