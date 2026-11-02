// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Glyphs to runs in reading order: runs per style and baseline, lines by vertical overlap,
//! columns by white space gutters, right-to-left lines in logical order and vertical writing
//! column by column. Nothing here reads the file; the glyphs come from the text device.

use std::rc::Rc;

use unicode_bidi::{BidiClass, Level, ParagraphBidiInfo, bidi_class};

use crate::layout::{Gap, Gaps, Run};
use crate::regions::{BASELINE_NEAR, Part, offset_baselines};
use crate::tables::{Rule, RuleKind};
use crate::text::{Gl, Link, Texts};

/// A gap wider than this many font sizes between two glyphs of a line separates two words.
const WORD_GAP: f32 = 0.15;

/// A gap wider than this many font sizes splits a line into pieces, which may stand in
/// different columns.
pub(crate) const PIECE_GAP: f32 = 0.8;

/// A column gutter is at least this many font sizes wide.
const GUTTER: f32 = 0.8;

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

/// Text that reads from right to left (Hebrew, Arabic and the like).
fn is_rtl(c: char) -> bool {
    matches!(bidi_class(c), BidiClass::R | BidiClass::AL)
}

/// White space that pads text: a space, a tab, the no-break spaces, the typographic spaces
/// and the ideographic space.
fn is_pad(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\u{a0}' | '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{3000}'
    )
}

/// A run being built: glyphs of one style next to each other on one baseline.
#[derive(Debug, Clone)]
struct Frag {
    text: String,
    /// Start on the baseline in the frame the run reads along +x: where its first glyph
    /// with ink starts, so white space the run starts with (padding) is not where it
    /// stands. A run of white space only starts at its first glyph.
    x: f32,
    y: f32,
    /// The same start in the page's frame.
    ox: f32,
    oy: f32,
    /// Where the first glyph starts, white space too: the end of the run before it when the
    /// run goes on it (see [`chain_heads`]).
    pen0: f32,
    /// The end of the last glyph with ink along the baseline: white space does not move
    /// it, so padding made of spaces leaves a gap.
    end: f32,
    /// The end of the last glyph, white space too.
    pen: f32,
    /// Whether a glyph with ink is in the run yet: white space before the first one joins
    /// it whatever the gap.
    inked: bool,
    /// Where a gap after the ink starts to count: the end of the first white space glyph
    /// after the last ink, or the ink end when none follows it yet. One space between words
    /// is never a gap, however wide the word spacing.
    free: f32,
    /// White space follows the last ink.
    padded: bool,
    /// The advance of the white space glyph the run starts with (0 when it starts with ink).
    lead: f32,
    size: f32,
    bold: bool,
    italic: bool,
    /// The family of the font of its first glyph (see [`crate::text::family_of`]).
    family: u32,
    mcid: Option<u32>,
    /// The URI link over the glyphs, an index into the page's links.
    link: Option<u32>,
    /// The drawn box the run's middle lies in, the innermost one, as an index into the
    /// page's boxes: a reading region.
    region: Option<u32>,
    /// The characters of the text from the first ink to the last, word spaces inside
    /// counted, padding before and after not: what [`gutter`] weighs as prose.
    chars: usize,
    /// Where the text of each glyph (and each space put in) starts in `text`: the units
    /// right-to-left reordering moves as a whole.
    units: Vec<u32>,
    /// The white space between words: where in `text` the word after it starts, the end of
    /// the ink before it and the start of the ink after it.
    gaps: Vec<(u32, f32, f32)>,
}

/// A frame that turns glyph positions so their text reads along +x.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Turn {
    #[default]
    None,
    /// Text that reads upward (rotated 90 degrees counterclockwise).
    Up,
    /// Text upside down.
    Half,
    /// Text that reads downward.
    Down,
}

impl Turn {
    /// The direction class of a unit vector, `None` when it is not near a quarter turn.
    pub(crate) fn of(dx: f32, dy: f32) -> Option<Turn> {
        const NEAR: f32 = 0.985;
        if dx >= NEAR {
            Some(Turn::None)
        } else if dy >= NEAR {
            Some(Turn::Up)
        } else if dx <= -NEAR {
            Some(Turn::Half)
        } else if dy <= -NEAR {
            Some(Turn::Down)
        } else {
            None
        }
    }

    /// How far the turned frame of a page `w` wide and `h` high moves so the page lies in
    /// `[0, w']` by `[0, h']` again, `w'` and `h'` its size in that frame.
    pub(crate) fn shift(self, w: f32, h: f32) -> (f32, f32) {
        match self {
            Turn::None => (0.0, 0.0),
            Turn::Up => (0.0, w),
            Turn::Half => (w, h),
            Turn::Down => (h, 0.0),
        }
    }

    /// The width and height of a page `w` wide and `h` high in the turned frame.
    pub(crate) fn size(self, w: f32, h: f32) -> (f32, f32) {
        match self {
            Turn::None | Turn::Half => (w, h),
            Turn::Up | Turn::Down => (h, w),
        }
    }

    /// A box (x0, y0, x1, y1) of a page `w` wide and `h` high, in the turned frame moved by
    /// [`Turn::shift`].
    pub(crate) fn place_box(self, b: [f32; 4], w: f32, h: f32) -> [f32; 4] {
        let (sx, sy) = self.shift(w, h);
        let (a, c) = (self.apply(b[0], b[1]), self.apply(b[2], b[3]));
        [
            a.0.min(c.0) + sx,
            a.1.min(c.1) + sy,
            a.0.max(c.0) + sx,
            a.1.max(c.1) + sy,
        ]
    }

    /// A point or a direction in the turned frame (directions are never moved).
    pub(crate) fn apply(self, x: f32, y: f32) -> (f32, f32) {
        match self {
            Turn::None => (x, y),
            Turn::Up => (y, -x),
            Turn::Half => (-x, -y),
            Turn::Down => (-y, x),
        }
    }
}

/// Builds the runs of a set of glyphs that all read along +x in the frame `turn`, in content
/// order: a new run at a change of style, line or a jump; a space inside a run at a word gap.
/// Each run also keeps where it starts in the frame `page` the page is read in.
fn fragments(glyphs: &[Gl], text: &Texts<'_>, turn: Turn, page: Turn) -> Vec<Frag> {
    // Sized once: a list grown step by step leaves blocks the allocator does not hand out
    // again for the next page.
    let mut out: Vec<Frag> = Vec::with_capacity(glyphs.len());
    for g in glyphs {
        let (x, y) = turn.apply(g.x, g.y);
        let t = text.of(g);
        let size = g.size.max(0.1);
        let adv = g.adv.unwrap_or(size * 0.5);
        if let Some(f) = out.last_mut() {
            let same_style = f.bold == g.bold
                && f.italic == g.italic
                && f.mcid == g.mcid
                && f.link == g.link
                && (f.size - size).abs() <= 0.05 * f.size.max(size);
            // White space joins the white space or ink before it; ink joins the ink before
            // it, so a wide stretch of white space between two words splits them.
            let pad = !t.is_empty() && t.chars().all(is_pad);
            let gap = x - if pad || !f.inked { f.pen } else { f.free };
            let on_line = (y - f.y).abs() <= 0.25 * f.size.max(size);
            if same_style && on_line && gap >= -0.5 * size && gap <= PIECE_GAP * size {
                let glued = g.joined
                    || f.text.ends_with(char::is_whitespace)
                    || t.starts_with(char::is_whitespace);
                let spaced = gap > WORD_GAP * size && !glued;
                if spaced {
                    f.units.push(f.text.len() as u32);
                    f.text.push(' ');
                }
                // Ink after white space starts a word: the gap before it is kept.
                if !pad && f.inked && (spaced || f.padded || t.starts_with(char::is_whitespace)) {
                    let lead = t.len() - t.trim_start().len();
                    f.gaps.push(((f.text.len() + lead) as u32, f.end, x));
                }
                if !t.is_empty() {
                    f.units.push(f.text.len() as u32);
                }
                f.text.push_str(t);
                f.pen = f.pen.max(x + adv);
                if pad {
                    if !f.padded {
                        f.free = f.free.max(x + adv);
                        f.padded = true;
                    }
                } else {
                    if !f.inked {
                        // The first ink: the run starts here, not at the white space.
                        f.x = x;
                        (f.ox, f.oy) = page.apply(g.x, g.y);
                    }
                    f.end = if f.inked { f.end.max(x + adv) } else { x + adv };
                    f.inked = true;
                    f.free = f.end;
                    f.padded = false;
                }
                continue;
            }
        }
        let (ox, oy) = page.apply(g.x, g.y);
        let pad = !t.is_empty() && t.chars().all(is_pad);
        out.push(Frag {
            text: t.to_string(),
            x,
            y,
            ox,
            oy,
            pen0: x,
            end: if pad { x } else { x + adv },
            pen: x + adv,
            inked: !pad,
            free: x + adv,
            padded: pad,
            lead: if pad { adv } else { 0.0 },
            size,
            bold: g.bold,
            italic: g.italic,
            family: g.family,
            mcid: g.mcid,
            link: g.link,
            region: None,
            chars: 0,
            units: vec![0],
            gaps: Vec::new(),
        });
    }
    // Padding is no text: only the characters from the first ink to the last count, so a
    // value padded to its place does not read as a line of prose.
    for f in &mut out {
        f.chars = f.text.trim_matches(is_pad).chars().count();
    }
    out
}

/// A row of fragments on one baseline, split where a gap is wide enough to be a column
/// gutter.
#[derive(Debug, Clone)]
struct Piece {
    row: usize,
    /// The start of the piece's first fragment with ink (of its first fragment when it has
    /// no ink).
    x0: f32,
    /// The end of the piece's last glyph, white space too; fragments of white space only
    /// count only in a piece without ink.
    x1: f32,
    /// Where a gap to the next fragment starts to count (see [`Frag::free`]), and whether
    /// white space ends the piece.
    free: f32,
    padded: bool,
    y0: f32,
    y1: f32,
    /// The baseline of the piece's first fragment.
    y: f32,
    size: f32,
    /// The characters of its fragments (see [`Frag::chars`]): padding is not counted.
    chars: usize,
    frags: Vec<usize>,
    /// The piece stands in for the text of a drawn box (the index of the box's group), read
    /// whole where the piece is read; its rows are counted in `rows`.
    stand: Option<usize>,
    rows: usize,
    /// A glyph with ink is in the piece: a piece of white space only has no size to align
    /// with the cell beside it.
    inked: bool,
    /// Every fragment with ink of the piece is bold (false for a piece without ink).
    bold: bool,
}

/// The white space between a run or piece whose gap counts from `free` (`padded` when white
/// space ends it) and the first ink of `f`, less one space: a gap of padding, not of a word
/// space.
fn gap(free: f32, padded: bool, f: &Frag) -> f32 {
    f.x - free - if padded { 0.0 } else { f.lead }
}

/// The vertical extent of a fragment: a quarter of its size below the baseline, three
/// quarters above.
fn extent(f: &Frag) -> (f32, f32) {
    (f.y - 0.25 * f.size, f.y + 0.75 * f.size)
}

/// For each fragment, the first of the chain it goes on in content order: a fragment that
/// goes on right where the one before it ends, on its baseline in its size and in its box (a
/// run cut at a link's edge or a change of style), moves with it, so other text that shares
/// the row is never put in between. Where it goes on counts from its first glyph, white space
/// too: a run cut at a change of style may start with a word space. A run parted by padding
/// (white space wider than a piece gap, a value padded to its place) starts a chain of its
/// own: a label drawn in that white space stands between the two in the order.
fn chain_heads(frags: &[Frag]) -> Vec<usize> {
    let mut head: Vec<usize> = Vec::with_capacity(frags.len());
    for (i, f) in frags.iter().enumerate() {
        let joins = i > 0 && {
            let p = &frags[i - 1];
            let size = p.size.max(f.size);
            p.region == f.region
                && (f.y - p.y).abs() <= 0.1 * size
                && (f.size - p.size).abs() <= 0.05 * size
                && (f.pen0 - p.pen).abs() <= 0.1 * size
                && !(p.padded && gap(p.free, p.padded, f) > PIECE_GAP * size)
        };
        head.push(if joins { head[i - 1] } else { i });
    }
    head
}

/// Rows of fragments that share a baseline (by vertical overlap), top to bottom, each split
/// into pieces at wide gaps. Fragments drawn twice in one place are dropped.
#[cfg(test)]
fn pieces(frags: &[Frag]) -> Vec<Piece> {
    let all: Vec<usize> = (0..frags.len()).collect();
    pieces_in(frags, &all, &chain_heads(frags), 0).0
}

/// [`pieces`] of the fragments `ids`, rows numbered from `base`, with the number of rows.
fn pieces_in(frags: &[Frag], ids: &[usize], head: &[usize], base: usize) -> (Vec<Piece>, usize) {
    let mut order: Vec<usize> = ids.to_vec();
    order.sort_by(|&a, &b| {
        frags[b]
            .y
            .total_cmp(&frags[a].y)
            .then(frags[a].x.total_cmp(&frags[b].x))
    });
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut core = (0.0f32, 0.0f32);
    for i in order {
        let (lo, hi) = extent(&frags[i]);
        if let Some(row) = rows.last_mut() {
            let overlap = hi.min(core.1) - lo.max(core.0);
            let smaller = (hi - lo).min(core.1 - core.0).max(0.01);
            if overlap / smaller >= 0.5 {
                row.push(i);
                continue;
            }
        }
        rows.push(vec![i]);
        core = (lo, hi);
    }
    let count = rows.len();
    let mut out = Vec::new();
    for (r, mut row) in rows.into_iter().enumerate() {
        row.sort_by(|&a, &b| frags[a].x.total_cmp(&frags[b].x));
        // The same text drawn again on top of itself (a fake bold) is read once: in x order
        // its copies stand next to each other.
        let mut kept: Vec<usize> = Vec::with_capacity(row.len());
        for i in row {
            let dup = kept.iter().rev().take(4).any(|&k| {
                let (a, b) = (&frags[k], &frags[i]);
                a.text == b.text
                    && (a.x - b.x).abs() < 0.3 * a.size
                    && (a.y - b.y).abs() < 0.1 * a.size
            });
            if !dup {
                kept.push(i);
            }
        }
        kept.sort_by(|&a, &b| {
            frags[head[a]]
                .x
                .total_cmp(&frags[head[b]].x)
                .then(head[a].cmp(&head[b]))
                .then(a.cmp(&b))
        });
        let mut piece: Option<Piece> = None;
        for i in kept {
            let f = &frags[i];
            let (lo, hi) = extent(f);
            if let Some(p) = piece.as_mut() {
                if gap(p.free, p.padded, f) <= PIECE_GAP * f.size.max(p.size) {
                    // A piece stands where its ink stands: a string of white space under the
                    // text (a field a print run blanks before it writes the value) or after
                    // it does not widen it.
                    if f.inked && !p.inked {
                        (p.x0, p.x1) = (f.x, f.pen.max(f.x));
                    } else if f.inked || !p.inked {
                        p.x1 = p.x1.max(f.pen);
                    }
                    if f.inked {
                        (p.free, p.padded) = (f.free, f.padded);
                    } else if !p.padded {
                        (p.free, p.padded) = (f.free, true);
                    }
                    p.y0 = p.y0.min(lo);
                    p.y1 = p.y1.max(hi);
                    p.size = p.size.max(f.size);
                    p.chars += f.chars;
                    if f.inked {
                        p.bold = f.bold && (p.bold || !p.inked);
                    }
                    p.inked |= f.inked;
                    p.frags.push(i);
                    continue;
                }
                out.push(piece.take().expect("a piece"));
            }
            piece = Some(Piece {
                row: base + r,
                x0: f.x,
                x1: f.pen.max(f.x),
                free: f.free,
                padded: f.padded,
                y0: lo,
                y1: hi,
                y: f.y,
                size: f.size,
                chars: f.chars,
                frags: vec![i],
                stand: None,
                rows: 1,
                inked: f.inked,
                bold: f.inked && f.bold,
            });
        }
        out.extend(piece);
    }
    (out, count)
}

/// The pieces in reading order: columns one after the other, with the pieces that cross the
/// gutter (titles, notes across the columns) splitting the columns into bands.
/// A horizontal drawn rule in the frame of the pieces: its height and its two ends.
pub(crate) type HRule = (f32, f32, f32);

/// A rule this share of a region's width or wider, across its gutter, parts its bands.
const WIDE_RULE: f32 = 2.0 / 3.0;

/// Pieces in reading order, and where the second side of a column gutter starts.
#[derive(Debug, Default)]
struct Order {
    ids: Vec<usize>,
    /// The places in `ids`, ascending, where the right side of a gutter of a lone row starts
    /// (see [`Split::Lone`]): the piece there starts a line even when it shares the row of
    /// the piece before it, which stands on the left side (a title beside the first line of
    /// a field list).
    sides: Vec<usize>,
}

impl Order {
    /// Whether the piece at place `k` starts the side of a gutter.
    fn starts_side(&self, k: usize) -> bool {
        self.sides.binary_search(&k).is_ok()
    }

    /// Puts `other` (an order of its own) at the end, its places of sides moved along.
    fn append(&mut self, other: Order) {
        let at = self.ids.len();
        self.sides.extend(other.sides.iter().map(|k| k + at));
        self.ids.extend(other.ids);
    }
}

fn reading_order(pieces: &[Piece], rules: &[HRule]) -> Order {
    let mut out = Order {
        ids: Vec::with_capacity(pieces.len()),
        sides: Vec::new(),
    };
    let all = (0..pieces.len()).collect();
    region(pieces, all, 0, rules, &mut out);
    out
}

fn region(p: &[Piece], mut ids: Vec<usize>, depth: usize, rules: &[HRule], out: &mut Order) {
    let deep = ids.len() > 3 && depth < COLUMN_DEPTH && ids.len() <= COLUMN_PIECES;
    let found = if deep { gutter(p, &ids) } else { None };
    // A gutter that only a drawn box makes is taken after the bands of white space across:
    // text above or below the boxes is not part of their columns.
    if let Some((_, _, Split::Boxed, _)) = found {
        ids.sort_by(|&a, &b| p[a].row.cmp(&p[b].row).then(p[a].x0.total_cmp(&p[b].x0)));
        if let Some(at) = widest_gap(p, &ids) {
            let rest = ids.split_off(at);
            region(p, ids, depth + 1, rules, out);
            region(p, rest, depth + 1, rules, out);
            return;
        }
    }
    if let Some((g0, g1, split, slack)) = found {
        ids.sort_by(|&a, &b| p[a].row.cmp(&p[b].row).then(p[a].x0.total_cmp(&p[b].x0)));
        let mut band: Vec<usize> = Vec::new();
        let flush = |band: &mut Vec<usize>, out: &mut Order| {
            if band.is_empty() {
                return;
            }
            let (left, right): (Vec<usize>, Vec<usize>) = band
                .drain(..)
                .partition(|&i| on_left(&p[i], (g0, g1), slack));
            region(p, left, depth + 1, rules, out);
            // A lone title and the line of the block whose row it shares are two lines. The
            // pieces of a row beside other gutters stay one line, as they read: cells of a
            // table in columns, labels of fields side by side.
            if split == Split::Lone {
                out.sides.push(out.ids.len());
            }
            region(p, right, depth + 1, rules, out);
        };
        // A drawn rule across the gutter and most of the region parts the bands too: the
        // text above it is read before the text below it (a titled block before a grid).
        let lo = ids.iter().map(|&i| p[i].x0).fold(f32::MAX, f32::min);
        let hi = ids.iter().map(|&i| p[i].x1).fold(f32::MIN, f32::max);
        let across: Vec<f32> = rules
            .iter()
            .filter(|r| r.1 < g0 && r.2 > g1 && r.2 - r.1 >= WIDE_RULE * (hi - lo))
            .map(|r| r.0)
            .collect();
        let mut low = f32::MAX;
        for id in ids {
            let ruled = across.iter().any(|&y| y < low && y > p[id].y1);
            if ruled {
                flush(&mut band, out);
                low = f32::MAX;
            }
            if crosses(&p[id], (g0, g1), slack) {
                flush(&mut band, out);
                out.ids.push(id);
                low = f32::MAX;
            } else {
                band.push(id);
                low = low.min(p[id].y0);
            }
        }
        flush(&mut band, out);
        return;
    }
    ids.sort_by(|&a, &b| p[a].row.cmp(&p[b].row).then(p[a].x0.total_cmp(&p[b].x0)));
    // Without a gutter, the region is cut at its widest band of white space across, and each
    // part is looked at again: columns may stand above or below full-width text.
    if ids.len() > 3
        && depth < COLUMN_DEPTH
        && ids.len() <= COLUMN_PIECES
        && let Some(at) = widest_gap(p, &ids)
    {
        let rest = ids.split_off(at);
        region(p, ids, depth + 1, rules, out);
        region(p, rest, depth + 1, rules, out);
        return;
    }
    out.ids.extend(ids);
}

/// A piece reaches into a gutter when it reaches past its edge by more than this share of
/// a step of the bands (see [`BAND_STEP`]): half a point at 8 points.
const GUTTER_SLACK: f32 = 0.5 * BAND_STEP;

/// Whether the piece `p` stands on the left side of the gutter from `g0` to `g1`: its end is
/// before the gutter, within `slack`. A piece of white space only has no ink to stand by: it
/// goes with the side its start lies on.
fn on_left(p: &Piece, (g0, g1): (f32, f32), slack: f32) -> bool {
    if p.inked {
        p.x1 <= g0 + slack
    } else {
        p.x0 < (g0 + g1) / 2.0
    }
}

/// Whether the piece `p` crosses the gutter from `g0` to `g1`: a piece with ink that reaches
/// into it from both sides by more than `slack`. White space crosses nothing: a blank string
/// that a print run draws under a row of fields is no text across the columns.
fn crosses(p: &Piece, (g0, g1): (f32, f32), slack: f32) -> bool {
    p.inked && p.x1 > g0 + slack && p.x0 < g1 - slack
}

/// Where to cut pieces sorted by row: after the row with the widest white space below it,
/// when that space is at least half a line high. The index is the first piece below the cut.
fn widest_gap(p: &[Piece], ids: &[usize]) -> Option<usize> {
    let mut best: Option<(f32, usize)> = None;
    let mut low = f32::MAX;
    for k in 1..ids.len() {
        let (prev, cur) = (&p[ids[k - 1]], &p[ids[k]]);
        low = low.min(prev.y0);
        if cur.row == prev.row {
            continue;
        }
        // The lowest point of everything above against the top of the next row.
        let top = ids[k..]
            .iter()
            .take_while(|&&i| p[i].row == cur.row)
            .map(|&i| p[i].y1)
            .fold(f32::MIN, f32::max);
        let gap = low - top;
        if gap >= 0.5 * prev.size.max(cur.size) && best.is_none_or(|(g, _)| gap > g) {
            best = Some((gap, k));
        }
    }
    best.map(|(_, k)| k)
}

/// What makes a column gutter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Split {
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

/// A block beside a lone row has at least this many rows: a lone cell beside fewer is a row
/// of a table, not a title.
const LONE_BLOCK: usize = 3;

/// Each step between the baselines of a block beside a lone row is within this share of
/// their median step: the block keeps one pitch, as the lines of a field list do.
const LONE_PITCH: f32 = 0.15;

/// Two text sizes differ when the larger is at least this many times the smaller (0.5 points
/// at 6 points): a ratio, so that it holds at every scale of the page.
pub(crate) const SIZE_STEP: f32 = 1.08;

/// A column gutter in a region: the widest vertical band of white space that only a few
/// pieces cross, with blocks on both sides that stand side by side: prose, lines whose
/// baselines do not line up, a lone row beside a block (see [`lone_row`]) or a drawn box; and
/// which of them makes it. Only
/// the best band is tried (see [`best_band`]): a band after it, where the best one fails, cuts
/// label and value blocks and table columns apart (ruling P6-X3). Pieces of white space only
/// are no text: they neither fill nor cross a band.
fn gutter(p: &[Piece], ids: &[usize]) -> Option<(f32, f32, Split, f32)> {
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
    let lo = inked.iter().map(|&i| p[i].x0).fold(f32::MAX, f32::min);
    let hi = inked.iter().map(|&i| p[i].x1).fold(f32::MIN, f32::max);
    if !(hi - lo).is_finite() || step.is_nan() || step <= 0.0 || hi - lo < 2.0 * step {
        return None;
    }
    // The span from the leftmost to the rightmost ink in a whole number of steps of about
    // `step`, so that a page printed larger or smaller has the same steps: a thousandth of a
    // step of float noise does not add a step.
    let width = (((hi - lo) / step - 1e-3).ceil().max(1.0) as usize).min(BAND_STEPS) + 2;
    let scale = (hi - lo) / (width - 2) as f32;
    let bin = |x: f32| (((x - lo) / scale) as usize).min(width - 1);
    let mut diff = vec![0i32; width + 1];
    for &i in inked {
        diff[bin(p[i].x0)] += 1;
        diff[bin(p[i].x1) + 1] -= 1;
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
) -> Option<Split> {
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
    let boxed = |side: &[usize]| side.iter().any(|&i| p[i].stand.is_some() && p[i].rows >= 2);
    // The baselines and sizes of a side, line by line: those of the pieces of each row.
    let baselines = |side: &[usize]| {
        let mut rows: Vec<(usize, f32, f32, bool)> = side
            .iter()
            .map(|&i| (p[i].row, p[i].y, p[i].size, p[i].inked))
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
        Split::Prose
    } else if apart() {
        Split::Offset
    } else if lone_row(p, &left, &right) || lone_row(p, &right, &left) {
        Split::Lone
    } else if !left.is_empty() && !right.is_empty() && (boxed(&left) || boxed(&right)) {
        Split::Boxed
    } else {
        return None;
    };
    // The columns stand side by side: their heights overlap.
    let span = |side: &[usize]| {
        (
            side.iter().map(|&i| p[i].y0).fold(f32::MAX, f32::min),
            side.iter().map(|&i| p[i].y1).fold(f32::MIN, f32::max),
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
    let inked =
        |side: &[usize]| -> Vec<usize> { side.iter().copied().filter(|&i| p[i].inked).collect() };
    let (one, mut block) = (inked(one), inked(block));
    let Some(&first) = one.first() else {
        return false;
    };
    if one.iter().any(|&i| p[i].row != p[first].row) || block.len() < LONE_BLOCK {
        return false;
    }
    // The baseline of each row of the block, top down: its highest one.
    block.sort_by_key(|&i| p[i].row);
    let mut rows: Vec<(usize, f32)> = Vec::new();
    for &i in &block {
        match rows.last_mut() {
            Some(r) if r.0 == p[i].row => r.1 = r.1.max(p[i].y),
            _ => rows.push((p[i].row, p[i].y)),
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
    let mut ys: Vec<f32> = block.iter().map(|&i| p[i].y).collect();
    ys.sort_by(f32::total_cmp);
    let near = BASELINE_NEAR * small;
    one.iter().all(|&i| {
        let y = p[i].y;
        let k = ys.partition_point(|b| *b < y - near);
        ys.get(k).is_none_or(|b| *b > y + near)
    })
}

/// Lines of fragments in reading order, as lists of fragment indices.
#[cfg(test)]
fn lines_of(frags: &[Frag], columns: bool) -> Vec<Vec<usize>> {
    lines_in(frags, columns, &[], &[])
}

/// Lines of fragments in reading order, as lists of fragment indices. A fragment tagged with
/// one of `boxes` (in the frame of the fragments) is read with the rest of its box: the box
/// stands as one piece among the others, ordered as they are, and its text is read whole
/// where it stands.
fn lines_in(frags: &[Frag], columns: bool, boxes: &[[f32; 4]], rules: &[HRule]) -> Vec<Vec<usize>> {
    let head = chain_heads(frags);
    let boxed = columns && frags.iter().any(|f| f.region.is_some());
    let order: Order;
    let ps: Vec<Piece>;
    if !boxed {
        let all: Vec<usize> = (0..frags.len()).collect();
        ps = pieces_in(frags, &all, &head, 0).0;
        order = if columns {
            reading_order(&ps, rules)
        } else {
            let mut o: Vec<usize> = (0..ps.len()).collect();
            o.sort_by(|&a, &b| {
                ps[a]
                    .row
                    .cmp(&ps[b].row)
                    .then(ps[a].x0.total_cmp(&ps[b].x0))
            });
            Order {
                ids: o,
                sides: Vec::new(),
            }
        };
    } else {
        (ps, order) = boxed_order(frags, &head, boxes, rules);
    }
    let mut lines: Vec<Vec<usize>> = Vec::new();
    let mut last_row = None;
    for (k, &i) in order.ids.iter().enumerate() {
        let p = &ps[i];
        // Two pieces of a row on the two sides of a gutter are two lines.
        if last_row == Some(p.row)
            && !order.starts_side(k)
            && let Some(l) = lines.last_mut()
        {
            l.extend(p.frags.iter().copied());
        } else {
            lines.push(p.frags.clone());
        }
        last_row = Some(p.row);
    }
    lines
}

/// The pieces of a page with drawn boxes, and their reading order. The pieces outside any
/// box and one stand-in piece for each box (its extent, its characters summed) are ordered as
/// a page without boxes is; each stand-in is then replaced by the pieces of its box in their
/// own reading order. Rows never join fragments of different boxes.
fn boxed_order(
    frags: &[Frag],
    head: &[usize],
    boxes: &[[f32; 4]],
    rules: &[HRule],
) -> (Vec<Piece>, Order) {
    // Free text first, then the fragments of each box, in the order the boxes are first met.
    let mut group_of: Vec<Option<usize>> = vec![None; boxes.len()];
    let mut groups: Vec<(u32, Vec<usize>)> = Vec::new();
    let mut free: Vec<usize> = Vec::new();
    for (i, f) in frags.iter().enumerate() {
        match f.region.filter(|&b| (b as usize) < boxes.len()) {
            Some(b) => {
                let g = *group_of[b as usize].get_or_insert_with(|| {
                    groups.push((b, Vec::new()));
                    groups.len() - 1
                });
                groups[g].1.push(i);
            }
            None => free.push(i),
        }
    }
    let (mut ps, free_rows) = pieces_in(frags, &free, head, 0);
    // The rows of the free text, top to bottom: their baselines and vertical extents.
    let mut bands: Vec<(f32, f32, f32)> = vec![(f32::MAX, f32::MAX, f32::MIN); free_rows];
    for p in &ps {
        let b = &mut bands[p.row];
        if b.0 == f32::MAX {
            b.0 = p.y;
        }
        b.1 = b.1.min(p.y0);
        b.2 = b.2.max(p.y1);
    }
    // The pieces of each box, rows numbered after all others for now; and its stand-in.
    let mut inner: Vec<std::ops::Range<usize>> = Vec::with_capacity(groups.len());
    let mut stands: Vec<Piece> = Vec::with_capacity(groups.len());
    let mut next = free_rows;
    for (g, (b, ids)) in groups.iter().enumerate() {
        let (list, rows) = pieces_in(frags, ids, head, next);
        let at = ps.len();
        let first = list.iter().filter(|p| p.row == next).collect::<Vec<_>>();
        let (lo, hi) = first.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
            (lo.min(p.y0), hi.max(p.y1))
        });
        let y = first.first().map_or(0.0, |p| p.y);
        let mut sizes: Vec<f32> = list.iter().map(|p| p.size).collect();
        sizes.sort_by(f32::total_cmp);
        let bx = boxes[*b as usize];
        stands.push(Piece {
            row: 0,
            x0: bx[0],
            x1: bx[2],
            free: bx[2],
            padded: false,
            // The first row's extent decides which row the stand-in shares; the box's own
            // extent is set once rows are numbered.
            y0: lo,
            y1: hi,
            y,
            size: sizes.get(sizes.len() / 2).copied().unwrap_or(1.0),
            chars: list.iter().map(|p| p.chars).sum(),
            frags: Vec::new(),
            stand: Some(g),
            rows,
            inked: true,
            bold: false,
        });
        ps.extend(list);
        inner.push(at..ps.len());
        next += rows;
    }
    // Rows of free text and stand-ins, top to bottom: a stand-in shares the row of free text
    // (or of another stand-in) its first row overlaps by half, else it has a row of its own.
    // Free rows keep their order.
    let shares = |a: (f32, f32), b: (f32, f32)| {
        let overlap = a.1.min(b.1) - a.0.max(b.0);
        let smaller = (a.1 - a.0).min(b.1 - b.0).max(0.01);
        overlap / smaller >= 0.5
    };
    let mut by_y: Vec<usize> = (0..stands.len()).collect();
    by_y.sort_by(|&a, &b| stands[b].y.total_cmp(&stands[a].y));
    // Each stand-in's place: after how many free rows, and the free row it shares if any.
    let mut place: Vec<(usize, Option<usize>)> = vec![(0, None); stands.len()];
    for &k in &by_y {
        let st = &stands[k];
        let at = bands.partition_point(|b| b.0 > st.y);
        let shared = [at.checked_sub(1), Some(at)]
            .into_iter()
            .flatten()
            .find(|&r| r < bands.len() && shares((bands[r].1, bands[r].2), (st.y0, st.y1)));
        place[k] = (at, shared);
    }
    // Number the rows: free rows in order, with the stand-ins that have rows of their own
    // put in before the free row they stand above.
    let mut row_of_free: Vec<usize> = Vec::with_capacity(free_rows);
    let mut stand_row: Vec<usize> = vec![0; stands.len()];
    let mut own: Vec<usize> = by_y
        .iter()
        .copied()
        .filter(|&k| place[k].1.is_none())
        .collect();
    own.sort_by(|&a, &b| {
        place[a]
            .0
            .cmp(&place[b].0)
            .then(stands[b].y.total_cmp(&stands[a].y))
    });
    let mut n = 0usize;
    let mut o = 0usize;
    // The band of the last row given out to a stand-in of its own, to let the next one share it.
    let mut last_own: Option<(usize, f32, f32)> = None;
    for r in 0..=free_rows {
        while o < own.len() && place[own[o]].0 == r {
            let st = &stands[own[o]];
            match last_own {
                Some((row, lo, hi)) if shares((lo, hi), (st.y0, st.y1)) => {
                    stand_row[own[o]] = row;
                }
                _ => {
                    stand_row[own[o]] = n;
                    last_own = Some((n, st.y0, st.y1));
                    n += 1;
                }
            }
            o += 1;
        }
        if r < free_rows {
            row_of_free.push(n);
            last_own = None;
            n += 1;
        }
    }
    for (k, pl) in place.iter().enumerate() {
        if let Some(r) = pl.1 {
            stand_row[k] = row_of_free[r];
        }
    }
    let base = n;
    let free_end = inner.first().map_or(ps.len(), |r| r.start);
    for p in &mut ps[..free_end] {
        p.row = row_of_free[p.row];
    }
    for (k, range) in inner.iter().enumerate() {
        let one_row = stands[k].rows == 1;
        for p in &mut ps[range.clone()] {
            // A box of one row reads in the row it shares: its text stays on that line.
            p.row = if one_row {
                stand_row[k]
            } else {
                base + p.row - free_rows
            };
        }
    }
    let first_stand = ps.len();
    for (k, mut st) in stands.into_iter().enumerate() {
        let bx = boxes[groups[k].0 as usize];
        (st.row, st.y0, st.y1) = (stand_row[k], bx[1], bx[3]);
        ps.push(st);
    }
    let top: Vec<usize> = (0..free_end.min(first_stand))
        .chain(first_stand..ps.len())
        .collect();
    let mut out = Order::default();
    region(&ps, top, 0, rules, &mut out);
    let mut order = Order {
        ids: Vec::with_capacity(ps.len()),
        sides: Vec::new(),
    };
    for (k, &id) in out.ids.iter().enumerate() {
        if out.starts_side(k) {
            order.sides.push(order.ids.len());
        }
        match ps[id].stand {
            Some(g) => {
                let mut boxed = Order::default();
                region(&ps, inner[g].clone().collect(), 0, rules, &mut boxed);
                order.append(boxed);
            }
            None => order.ids.push(id),
        }
    }
    (ps, order)
}

/// The fragments of a line from the first one with ink on: runs of white space only before
/// it are padding, not text. A line without ink stays whole. The padding inside the first
/// fragment with ink is trimmed in [`line_runs`].
fn from_first_ink<'a>(frags: &[Frag], ids: &'a [usize]) -> &'a [usize] {
    let first = ids.iter().position(|&i| frags[i].inked).unwrap_or(0);
    &ids[first..]
}

/// The runs of one line, numbered `line`. A run that starts a new word gets a leading space;
/// white space at the start of the line is dropped; a line with right-to-left text becomes
/// one run in logical order. A run's link is the one of `links` its glyphs lie in.
fn line_runs(page: u32, frags: &[Frag], ids: &[usize], line: u32, links: &[Link]) -> Vec<Run> {
    let n = ids.len();
    // For each fragment: the part of its text that is kept, whether its padding is written as
    // one space after it, and whether a space is put in before it.
    let mut keep: Vec<(usize, usize)> = ids.iter().map(|&i| (0, frags[i].text.len())).collect();
    let mut pad_after = vec![false; n];
    let mut spaced = vec![false; n];
    // White space at the start of a line (a value padded into its place) is not text: the
    // line starts at its first ink (see [`from_first_ink`]).
    if let Some(&first) = ids.first()
        && frags[first].inked
    {
        let t = &frags[first].text;
        keep[0].0 = t.len() - t.trim_start_matches(is_pad).len();
    }
    let rtl = ids.iter().any(|&i| frags[i].text.chars().any(is_rtl));
    for pos in 1..n {
        let (p, f) = (&frags[ids[pos - 1]], &frags[ids[pos]]);
        // White space that opens a piece gap is padding: it is written as one space.
        if gap(p.free, p.padded, f) > PIECE_GAP * f.size.max(p.size) {
            let lead = f.text.len() - f.text.trim_start_matches(is_pad).len();
            let trail = p.text.trim_end_matches(is_pad).len().max(keep[pos - 1].0);
            if lead > 0 || trail < keep[pos - 1].1 {
                keep[pos - 1].1 = trail.min(keep[pos - 1].1);
                pad_after[pos - 1] = true;
                keep[pos].0 = lead;
            }
        }
        let glued = pad_after[pos - 1]
            || p.text.ends_with(char::is_whitespace)
            || f.text[keep[pos].0..].starts_with(char::is_whitespace);
        // A gap after the run before is a word space. So is a run that stands wholly left of
        // where the run before starts: a run that moves with the chain of the run before it
        // (cut at a change of style or a link's edge, see [`chain_heads`]) is put before text
        // drawn over it, and that text goes on after it, never as one word with it. A run
        // parted by padding starts a chain of its own and stands in its place. A run over the
        // start of the run before (an accent drawn on its own over a first letter) joins it. A
        // line with right-to-left text keeps its visual order here, so only the gap counts
        // there.
        let size = f.size.max(p.size);
        let before = f.x < p.x && f.end <= p.x + WORD_GAP * size;
        if !glued && (f.x - p.end > WORD_GAP * size || (!rtl && before)) {
            spaced[pos] = true;
        }
    }
    // Padding at the end of a line (a label whose value is empty) opens a gap to nothing: it
    // is dropped. One space there stays as drawn.
    if let Some(&last) = ids.last() {
        let t = &frags[last].text;
        let (a, b) = keep[n - 1];
        let kept = t[..b].trim_end_matches(is_pad).len().max(a);
        if t[kept..b].chars().count() > 1 {
            keep[n - 1].1 = kept;
        }
    }
    let uri = |f: &Frag| {
        f.link
            .and_then(|k| links.get(k as usize))
            .map(|(_, u)| u.clone())
    };
    let run_of = |f: &Frag, text: String| Run {
        page,
        text,
        x: f.x,
        y: f.y,
        w: (f.end - f.x).max(0.0),
        h: f.size,
        size: f.size,
        bold: f.bold,
        italic: f.italic,
        family: f.family,
        mcid: crate::layout::kept_mcid(f.mcid),
        uri: uri(f),
        line,
        off_axis: false,
        note: false,
        gaps: Gaps::default(),
    };
    if !rtl {
        return (0..n)
            .map(|pos| {
                let f = &frags[ids[pos]];
                let (a, b) = keep[pos];
                let mut text = String::with_capacity(b.saturating_sub(a) + 2);
                if spaced[pos] {
                    text.push(' ');
                }
                text.push_str(&f.text[a..b.max(a)]);
                if pad_after[pos] {
                    text.push(' ');
                }
                // The gaps inside the kept text, by the words before each.
                let kept = &f.text[a..b.max(a)];
                let gaps = Gaps::new(
                    f.gaps
                        .iter()
                        .filter(|g| (g.0 as usize) > a && (g.0 as usize) < b)
                        .map(|&(at, from, to)| Gap {
                            words: kept[..at as usize - a].split_whitespace().count() as u32,
                            from,
                            to,
                        })
                        .collect(),
                );
                Run {
                    gaps,
                    ..run_of(f, text)
                }
            })
            .collect();
    }
    // The glyphs stand in visual order. Reordering a visual line by the bidi rules gives the
    // logical order back; the text of one glyph (a ligature mapped to several characters)
    // moves as a whole. Each unit keeps the fragment it comes from, so the runs of the line
    // are cut where its link changes, in logical order.
    let mut units: Vec<(&str, usize)> = Vec::new();
    for pos in 0..n {
        let f = &frags[ids[pos]];
        let (a, b) = keep[pos];
        if spaced[pos] {
            units.push((" ", pos));
        }
        let mut starts: Vec<usize> = f.units.iter().map(|&u| u as usize).collect();
        starts.push(f.text.len());
        for w in starts.windows(2) {
            let (u0, u1) = (w[0].max(a), w[1].min(b));
            if u1 > u0 {
                units.push((&f.text[u0..u1], pos));
            }
        }
        if pad_after[pos] {
            units.push((" ", pos));
        }
    }
    let texts: Vec<&str> = units.iter().map(|u| u.0).collect();
    let mut out: Vec<Run> = Vec::new();
    // The fragments of the run being built, for its extent.
    let mut span: (f32, f32) = (f32::MAX, f32::MIN);
    for k in logical_indices(&texts) {
        let (t, pos) = units[k];
        let f = &frags[ids[pos]];
        match out.last_mut() {
            Some(r) if r.uri.as_ref().map(Rc::as_ptr) == uri(f).as_ref().map(Rc::as_ptr) => {
                r.text.push_str(t);
            }
            _ => {
                if let Some(r) = out.last_mut() {
                    (r.x, r.w) = (span.0, span.1 - span.0);
                }
                span = (f32::MAX, f32::MIN);
                out.push(run_of(f, t.to_string()));
            }
        }
        span = (span.0.min(f.x), span.1.max(f.end));
    }
    if let Some(r) = out.last_mut() {
        (r.x, r.w) = (span.0, span.1 - span.0);
    }
    out
}

/// The units of a visual line in logical order. In the bidi algorithm each unit stands for
/// its first strongly directed character; a unit with text of both directions is already in
/// logical order inside and stands for a space, taking the direction around it.
#[cfg(test)]
fn logical_order(units: &[&str]) -> String {
    logical_indices(units)
        .into_iter()
        .filter_map(|i| units.get(i).copied())
        .collect()
}

/// The indices of `units`, a visual line, in logical order (see [`logical_order`]).
fn logical_indices(units: &[&str]) -> Vec<usize> {
    let stand_in: String = units
        .iter()
        .map(|u| {
            let rtl = u.chars().any(is_rtl);
            let ltr = u.chars().any(|c| bidi_class(c) == BidiClass::L);
            if rtl && ltr {
                return ' ';
            }
            u.chars()
                .find(|&c| matches!(bidi_class(c), BidiClass::L | BidiClass::R | BidiClass::AL))
                .or_else(|| u.chars().next())
                .unwrap_or(' ')
        })
        .collect();
    let rtl = stand_in.chars().filter(|&c| is_rtl(c)).count();
    let ltr = stand_in
        .chars()
        .filter(|&c| bidi_class(c) == BidiClass::L)
        .count();
    let level = if rtl > ltr {
        Level::rtl()
    } else {
        Level::ltr()
    };
    let info = ParagraphBidiInfo::new(&stand_in, Some(level));
    let levels = info.reordered_levels_per_char(0..stand_in.len());
    ParagraphBidiInfo::reorder_visual(&levels)
        .into_iter()
        .filter(|&i| i < units.len())
        .collect()
}

/// Vertical writing: one run per column, glyphs top to bottom, columns from right to left.
fn vertical_runs(
    page: u32,
    glyphs: &[Gl],
    text: &Texts<'_>,
    turn: Turn,
    first_line: u32,
) -> Vec<Run> {
    let mut cols: Vec<(Run, f32)> = Vec::new();
    for g in glyphs {
        let t = text.of(g);
        let size = g.size.max(0.1);
        let (x, y) = turn.apply(g.x, g.y);
        if let Some((c, bottom)) = cols.last_mut()
            && (x - c.x).abs() <= 0.5 * size
            && y <= *bottom + 0.1 * size
            && *bottom - y <= 2.0 * size
        {
            c.text.push_str(t);
            *bottom = y;
            c.h = c.h.max(size);
            continue;
        }
        cols.push((
            Run {
                page,
                text: t.to_string(),
                x,
                y,
                w: size,
                h: size,
                size,
                bold: g.bold,
                italic: g.italic,
                family: g.family,
                mcid: crate::layout::kept_mcid(g.mcid),
                uri: None,
                line: 0,
                off_axis: true,
                note: false,
                gaps: Gaps::default(),
            },
            y,
        ));
    }
    let mut runs: Vec<Run> = cols.into_iter().map(|(r, _)| r).collect();
    runs.sort_by(|a, b| b.x.total_cmp(&a.x).then(b.y.total_cmp(&a.y)));
    for (k, r) in runs.iter_mut().enumerate() {
        r.line = first_line + k as u32;
    }
    runs
}

/// Runs in reading order for one set of glyphs (visible or invisible), with the frame turn
/// used: the main text along +x first, then text in other directions, then vertical writing.
pub(crate) fn runs_of(
    page: u32,
    glyphs: &[Gl],
    text: &Texts<'_>,
    turn: Turn,
    links: &[Link],
) -> Vec<Run> {
    runs_in(page, glyphs, text, turn, links, &[], &[]).0
}

/// [`runs_of`], with the drawn boxes of the page (in the frame of the page box) as reading
/// regions for the main text: the text of a box is read whole. Also says why the boxes were
/// not used, when a bound kept them out. A run keeps its [`Gaps`] only where an end or side
/// of one of the page's drawn `rules` lies inside it along the main text: only a column rule
/// may cut a run.
pub(crate) fn runs_in(
    page: u32,
    glyphs: &[Gl],
    text: &Texts<'_>,
    turn: Turn,
    links: &[Link],
    boxes: &[[f32; 4]],
    rules: &[Rule],
) -> (Vec<Run>, Option<String>) {
    let mut main = Vec::with_capacity(glyphs.len());
    let mut vertical = Vec::new();
    let mut others: Vec<(Turn, Vec<Gl>)> = Vec::new();
    for g in glyphs {
        if g.vertical {
            vertical.push(*g);
            continue;
        }
        let (dx, dy) = turn.apply(g.dx, g.dy);
        match Turn::of(dx, dy) {
            Some(Turn::None) => main.push(*g),
            dir => {
                // Glyphs at an odd angle read in the frame of the nearest quarter turn.
                let t = dir.unwrap_or(if dx.abs() >= dy.abs() {
                    if dx >= 0.0 { Turn::None } else { Turn::Half }
                } else if dy >= 0.0 {
                    Turn::Up
                } else {
                    Turn::Down
                });
                if t == Turn::None {
                    main.push(*g);
                } else if let Some((_, list)) = others.iter_mut().find(|(d, _)| *d == t) {
                    list.push(*g);
                } else {
                    others.push((t, vec![*g]));
                }
            }
        }
    }
    let mut runs = Vec::with_capacity(main.len());
    let mut line = 0u32;
    let mut frags = fragments(&main, text, turn, turn);
    let turned: Vec<[f32; 4]> = boxes
        .iter()
        .map(|b| {
            let (a, c) = (turn.apply(b[0], b[1]), turn.apply(b[2], b[3]));
            [a.0.min(c.0), a.1.min(c.1), a.0.max(c.0), a.1.max(c.1)]
        })
        .collect();
    let mut columns: Vec<f32> = rules
        .iter()
        .flat_map(|r| [turn.apply(r.x0, r.y0).0, turn.apply(r.x1, r.y1).0])
        .collect();
    columns.sort_by(f32::total_cmp);
    // The horizontal rules in the frame of the main text, which part bands of columns.
    let across: Vec<HRule> = rules
        .iter()
        .flat_map(|r| {
            let (a, b) = (turn.apply(r.x0, r.y0), turn.apply(r.x1, r.y1));
            let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
            match r.kind {
                // A rectangle's top and bottom are rules across too.
                RuleKind::Rect => vec![(a.1, x0, x1), (b.1, x0, x1)],
                // A line rule runs along one axis by how it is made: flat in this frame when
                // it is wider than high.
                RuleKind::Line if (a.1 - b.1).abs() < (a.0 - b.0).abs() => {
                    vec![((a.1 + b.1) / 2.0, x0, x1)]
                }
                RuleKind::Line => Vec::new(),
            }
        })
        .collect();
    let unused = if turned.is_empty() {
        None
    } else {
        tag_regions(&mut frags, &turned).err()
    };
    for ids in lines_in(&frags, true, &turned, &across) {
        let mut r = line_runs(page, &frags, from_first_ink(&frags, &ids), line, links);
        for run in &mut r {
            let k = columns.partition_point(|x| *x <= run.x);
            if !columns.get(k).is_some_and(|x| *x < run.x + run.w) {
                run.gaps = Gaps::default();
            }
        }
        runs.extend(r);
        line += 1;
    }
    for (dir, list) in others {
        // Lines are found in the frame where this text reads along +x; the runs keep the
        // position of their first glyph in the page's frame.
        let frame = compose(turn, dir);
        let frags = fragments(&list, text, frame, turn);
        for ids in lines_in(&frags, false, &[], &[]) {
            let ids = from_first_ink(&frags, &ids);
            let mut r = line_runs(page, &frags, ids, line, links);
            for (run, &i) in r.iter_mut().zip(ids) {
                (run.x, run.y) = (frags[i].ox, frags[i].oy);
                run.off_axis = true;
                run.gaps = Gaps::default();
            }
            runs.extend(r);
            line += 1;
        }
    }
    runs.extend(vertical_runs(page, &vertical, text, turn, line));
    (runs, unused)
}

/// Fragments drawn before one that are looked at for the copy it may be of.
const TWIN_LOOK: usize = 8;

/// Tags each fragment with the innermost of `boxes` its middle lies in (see
/// [`crate::regions::regions_of`]; a middle on a box's edge lies in neither box beside it). Past the bounds no fragment is tagged, and the reason is
/// given.
fn tag_regions(frags: &mut [Frag], boxes: &[[f32; 4]]) -> Result<(), String> {
    // The middle of the ink: padding on either side does not move the run into the box
    // beside it.
    let points: Vec<(f32, f32, f32)> = frags
        .iter()
        .map(|f| ((f.x + f.end.max(f.x)) / 2.0, f.y + 0.25 * f.size, f.size))
        .collect();
    match crate::regions::regions_of(&points, boxes) {
        Ok(regions) => {
            for (f, r) in frags.iter_mut().zip(regions) {
                f.region = r;
            }
            // A copy drawn a little apart (a fake bold) is read with the first copy, in its
            // box, so it is dropped as a copy there even where a box's edge parts the two.
            for k in 1..frags.len() {
                let twin = (k.saturating_sub(TWIN_LOOK)..k).rev().find(|&j| {
                    let (a, b) = (&frags[j], &frags[k]);
                    a.text == b.text
                        && (a.x - b.x).abs() < 0.3 * a.size
                        && (a.y - b.y).abs() < 0.1 * a.size
                });
                if let Some(j) = twin {
                    frags[k].region = frags[j].region;
                }
            }
            Ok(())
        }
        Err(why) => {
            frags.iter_mut().for_each(|f| f.region = None);
            Err(why)
        }
    }
}

/// The frame that first turns by `a` and then reads text of direction `b` (in the frame `a`)
/// along +x.
fn compose(a: Turn, b: Turn) -> Turn {
    let q = |t: Turn| match t {
        Turn::None => 0,
        Turn::Up => 1,
        Turn::Half => 2,
        Turn::Down => 3,
    };
    match (q(a) + q(b)) % 4 {
        0 => Turn::None,
        1 => Turn::Up,
        2 => Turn::Half,
        _ => Turn::Down,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A piece of ink from `x0` to `x1` in row `row`, of 6 points.
    fn ink_piece(x0: f32, x1: f32, row: usize) -> Piece {
        Piece {
            row,
            x0,
            x1,
            free: x1,
            padded: false,
            y0: 700.0 - 10.0 * row as f32,
            y1: 706.0 - 10.0 * row as f32,
            y: 701.5 - 10.0 * row as f32,
            size: 6.0,
            chars: 4,
            frags: Vec::new(),
            stand: None,
            rows: 1,
            inked: true,
            bold: false,
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
    fn right_to_left_line_keeps_ligature_glyphs_whole() {
        // "salam" drawn from right to left, with lam and alef in one ligature glyph whose
        // text is already in logical order.
        let visual = ["\u{645}", "\u{644}\u{627}", "\u{633}"];
        assert_eq!(logical_order(&visual), "\u{633}\u{644}\u{627}\u{645}");
        // A Latin word next to it keeps its own order.
        let visual = ["a", "b", " ", "\u{645}", "\u{633}"];
        assert_eq!(logical_order(&visual), "ab \u{633}\u{645}");
    }

    fn frag(text: &str, x: f32, y: f32) -> Frag {
        Frag {
            text: text.into(),
            x,
            y,
            ox: x,
            oy: y,
            pen0: x,
            end: x + 6.0 * text.chars().count() as f32,
            pen: x + 6.0 * text.chars().count() as f32,
            inked: true,
            free: x + 6.0 * text.chars().count() as f32,
            padded: false,
            lead: 0.0,
            size: 10.0,
            bold: false,
            italic: false,
            family: 0,
            mcid: None,
            link: None,
            region: None,
            chars: text.chars().count(),
            units: vec![0],
            gaps: Vec::new(),
        }
    }

    #[test]
    fn two_columns_with_a_title_and_a_footer() {
        let mut frags = vec![frag("A title across both of the columns", 100.0, 800.0)];
        for k in 0..6 {
            let y = 700.0 - 12.0 * k as f32;
            frags.push(frag(&format!("left column line {k} words"), 50.0, y));
            frags.push(frag(&format!("right column line {k} words"), 320.0, y));
        }
        frags.push(frag(
            "A footer note across both of the columns here",
            100.0,
            40.0,
        ));
        let lines = lines_of(&frags, true);
        let text: Vec<String> = lines
            .iter()
            .map(|l| l.iter().map(|&i| frags[i].text.clone()).collect())
            .collect();
        assert_eq!(text[0], "A title across both of the columns");
        assert_eq!(text[1], "left column line 0 words");
        assert_eq!(text[6], "left column line 5 words");
        assert_eq!(text[7], "right column line 0 words");
        assert_eq!(text[13], "A footer note across both of the columns here");
    }

    #[test]
    fn a_run_cut_at_a_link_edge_stays_together_in_its_row() {
        // Small text of a figure shares the row of a caption; both are cut at the edges of a
        // link box. Each text stays whole, in the order of where it starts.
        let part = |text: &str, x: f32, end: f32, y: f32, size: f32, link: Option<u32>| Frag {
            end,
            pen: end,
            free: end,
            size,
            link,
            ..frag(text, x, y)
        };
        let frags = vec![
            part("small text wit", 329.6, 397.0, 415.8, 3.0, Some(0)),
            part("h more", 397.0, 516.9, 415.8, 3.0, None),
            part("Figure 2: (", 108.0, 295.0, 412.6, 10.0, None),
            part("example.org", 295.0, 396.3, 412.6, 10.0, Some(0)),
            part(") left", 396.3, 504.0, 412.6, 10.0, None),
        ];
        let lines = lines_of(&frags, true);
        assert_eq!(lines, [vec![2, 3, 4, 0, 1]]);
    }

    #[test]
    fn a_line_drawn_twice_in_cut_runs_is_read_once() {
        // Five runs of one line (alternating bold), drawn again 0.2 points to the right as a
        // fake bold: each copy is a chain of its own, and the copy is still dropped.
        let mut frags = Vec::new();
        for shift in [0.0, 0.2] {
            for k in 0..5 {
                let x = 100.0 + 30.0 * k as f32 + shift;
                frags.push(Frag {
                    bold: k % 2 == 0,
                    end: x + 30.0,
                    pen: x + 30.0,
                    free: x + 30.0,
                    ..frag(&format!("part{k}"), x, 700.0)
                });
            }
        }
        let lines = lines_of(&frags, true);
        assert_eq!(lines, [vec![0, 1, 2, 3, 4]]);
    }

    #[test]
    fn padding_splits_pieces_but_not_words() {
        // A label, five no-break spaces of 6 points, a value: the gap counts from the end of
        // the first space, so two pieces, and the padding is written as one space.
        let mut label = frag("Points:", 50.0, 700.0);
        label.text.push_str("\u{a0}\u{a0}\u{a0}\u{a0}\u{a0}");
        (label.pen, label.free, label.padded) = (122.0, 98.0, true);
        let value = frag("1200", 122.0, 700.0);
        let ps = pieces(&[label.clone(), value.clone()]);
        assert_eq!(ps.len(), 2, "{ps:?}");
        // The piece of the label reaches over its padding.
        assert_eq!(ps[0].x1, 122.0);
        let runs = line_runs(1, &[label, value], &[0, 1], 0, &[]);
        let text: Vec<&str> = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, ["Points: ", "1200"]);
        // One space and wide word spacing (5 points at 10) is no gap: one piece, and two
        // spaces inside a run stay.
        let mut see = frag("see  ", 50.0, 700.0);
        (see.end, see.free, see.padded) = (68.0, 71.0, true);
        let ps = pieces(&[see.clone(), frag("www", 76.0, 700.0)]);
        assert_eq!(ps.len(), 1, "{ps:?}");
        let runs = line_runs(1, &[see, frag("www", 76.0, 700.0)], &[0, 1], 0, &[]);
        assert_eq!(runs[0].text, "see  ");
        assert!(is_pad('\u{3000}') && is_pad('\u{202f}') && is_pad('\t'));
        // Padding at the end of a line is dropped.
        let runs = line_runs(
            1,
            &[frag("Codes:\u{a0}\u{a0}\u{a0}", 50.0, 700.0)],
            &[0],
            0,
            &[],
        );
        assert_eq!(runs[0].text, "Codes:");
    }

    /// A run put after the run it goes on in reading order gets a word space only when it
    /// starts well left of that run: a label drawn in the padding of a value run that the chain
    /// order puts before it. An accent drawn as a run of its own over the first letter of a
    /// word, a little left of the word's start or inside it, joins the word.
    #[test]
    fn a_run_left_of_the_run_before_is_spaced_only_when_well_left() {
        let text = |a: Frag, b: Frag| -> String {
            line_runs(1, &[a, b], &[0, 1], 0, &[])
                .iter()
                .map(|r| r.text.as_str())
                .collect()
        };
        assert_eq!(
            text(frag("Uber", 100.0, 700.0), frag("\u{a8}", 99.6, 700.0)),
            "Uber\u{a8}"
        );
        assert_eq!(
            text(frag("Uber", 100.0, 700.0), frag("\u{a8}", 101.0, 700.0)),
            "Uber\u{a8}"
        );
        // Ten sizes left: the label `Kranken` in the padding of a run of values.
        assert_eq!(
            text(frag("98,76", 200.0, 700.0), frag("Kranken", 100.0, 700.0)),
            "98,76 Kranken"
        );
    }

    /// Glyphs of `parts` (text, bold) in a fixed-pitch font of `size` with an advance of
    /// `adv`, one glyph per character from `x` on the baseline `y`, and the page text they
    /// point into.
    fn pitched(parts: &[(&str, bool)], x: f32, y: f32, size: f32, adv: f32) -> (String, Vec<Gl>) {
        let mut text = String::new();
        let mut glyphs = Vec::new();
        let mut at = x;
        for &(part, bold) in parts {
            for c in part.chars() {
                glyphs.push(Gl {
                    start: text.len() as u32,
                    len: c.len_utf8() as u32,
                    x: at,
                    y,
                    dx: 1.0,
                    dy: 0.0,
                    size,
                    adv: Some(adv),
                    bold,
                    italic: false,
                    family: 0,
                    mcid: None,
                    invisible: false,
                    layer: None,
                    vertical: false,
                    joined: false,
                    actual: None,
                    link: None,
                });
                text.push(c);
                at += adv;
            }
        }
        (text, glyphs)
    }

    #[test]
    fn leading_padding_is_not_the_start() {
        // A data line of a print run: six spaces of 5.7 points, then the value. The value
        // starts at its first ink, 40 + 6 * 5.7 points.
        let (text, glyphs) = pitched(&[("      Muster GmbH", false)], 40.0, 700.0, 9.5, 5.7);
        let texts = Texts {
            text: &text,
            actuals: &[],
        };
        let frags = fragments(&glyphs, &texts, Turn::None, Turn::None);
        assert_eq!(frags.len(), 1, "{frags:?}");
        let ps = pieces(&frags);
        assert!((ps[0].x0 - 74.2).abs() < 0.01, "{ps:?}");
        let runs = runs_of(1, &glyphs, &texts, Turn::None, &[]);
        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!(runs[0].text, "Muster GmbH");
        assert!((runs[0].x - 74.2).abs() < 0.01, "{runs:?}");
        assert!((runs[0].w - 11.0 * 5.7).abs() < 0.01, "{runs:?}");
        // Padding drawn in another style is a run of its own: the line starts after it too.
        let (text, glyphs) = pitched(&[("    ", true), ("Erika", false)], 40.0, 700.0, 9.5, 5.7);
        let texts = Texts {
            text: &text,
            actuals: &[],
        };
        let runs = runs_of(1, &glyphs, &texts, Turn::None, &[]);
        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!(runs[0].text, "Erika");
        assert!((runs[0].x - 62.8).abs() < 0.01, "{runs:?}");
        // Text that reads upward keeps the start of its ink in the page's frame too.
        let (text, mut glyphs) = pitched(&[("      Muster GmbH", false)], 40.0, 0.0, 9.5, 5.7);
        for g in &mut glyphs {
            (g.x, g.y, g.dx, g.dy) = (100.0, g.x, 0.0, 1.0);
        }
        let texts = Texts {
            text: &text,
            actuals: &[],
        };
        let runs = runs_of(1, &glyphs, &texts, Turn::None, &[]);
        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!(runs[0].text, "Muster GmbH");
        assert!(runs[0].off_axis, "{runs:?}");
        assert!((runs[0].x - 100.0).abs() < 0.01, "{runs:?}");
        assert!((runs[0].y - 74.2).abs() < 0.01, "{runs:?}");
    }

    #[test]
    fn padding_is_not_counted_as_text() {
        // A value padded on both sides: only its six characters count as text.
        let (text, glyphs) = pitched(&[("      12.345      ", false)], 40.0, 700.0, 9.5, 5.7);
        let texts = Texts {
            text: &text,
            actuals: &[],
        };
        let frags = fragments(&glyphs, &texts, Turn::None, Turn::None);
        assert_eq!(frags.len(), 1, "{frags:?}");
        assert_eq!(frags[0].chars, 6);
        assert_eq!(pieces(&frags)[0].chars, 6);
    }

    /// Guard: a run cut at a change of style that begins with a word space goes on the run
    /// before it, from where that one's pen ends.
    #[test]
    fn style_change_after_a_space_stays_chained() {
        let (text, glyphs) = pitched(&[("plain", false), (" bold", true)], 40.0, 700.0, 10.0, 6.0);
        let texts = Texts {
            text: &text,
            actuals: &[],
        };
        let frags = fragments(&glyphs, &texts, Turn::None, Turn::None);
        assert_eq!(frags.len(), 2, "{frags:?}");
        assert_eq!(chain_heads(&frags), [0, 0]);
        let runs = runs_of(1, &glyphs, &texts, Turn::None, &[]);
        let line: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(line, "plain bold");
    }

    #[test]
    fn label_value_rows_are_read_row_by_row() {
        let mut frags = Vec::new();
        for (k, (label, value)) in [
            ("Name", "Ann"),
            ("Age", "31"),
            ("Town", "Bern"),
            ("Zip", "3000"),
        ]
        .iter()
        .enumerate()
        {
            let y = 700.0 - 14.0 * k as f32;
            frags.push(frag(label, 50.0, y));
            frags.push(frag(value, 300.0, y));
        }
        let lines = lines_of(&frags, true);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], [0, 1]);
    }

    #[test]
    fn offset_baselines_make_columns() {
        // Two blocks of short lines side by side whose baselines do not line up (an address
        // and a column of document facts): each block reads whole.
        let mut frags = Vec::new();
        for (k, t) in ["Sample Ltd", "Main Road 1", "1000 Town", "Country"]
            .iter()
            .enumerate()
        {
            frags.push(frag(t, 50.0, 700.0 - 14.0 * k as f32));
        }
        for (k, t) in ["Invoice 4711", "Date 2030", "Customer 9", "Page 1", "Euro"]
            .iter()
            .enumerate()
        {
            frags.push(frag(t, 300.0, 703.0 - 11.6 * k as f32));
        }
        let lines = lines_of(&frags, true);
        let want: Vec<Vec<usize>> = (0..9).map(|i| vec![i]).collect();
        assert_eq!(lines, want);
    }

    #[test]
    fn boxes_are_read_whole() {
        // Two boxes side by side whose lines share their baselines, a title above both and a
        // line below them: each box is read whole, left box first.
        let boxes = [[40.0, 600.0, 240.0, 720.0], [290.0, 600.0, 520.0, 720.0]];
        let mut frags = vec![frag("A title over both boxes", 40.0, 760.0)];
        for k in 0..3 {
            let y = 700.0 - 14.0 * k as f32;
            let mut l = frag(&format!("left {k}"), 50.0, y);
            l.region = Some(0);
            frags.push(l);
            let mut r = frag(&format!("right {k}"), 300.0, y);
            r.region = Some(1);
            frags.push(r);
        }
        frags.push(frag("A line under the boxes", 40.0, 560.0));
        let lines = lines_in(&frags, true, &boxes, &[]);
        let text: Vec<String> = lines
            .iter()
            .map(|l| l.iter().map(|&i| frags[i].text.clone()).collect())
            .collect();
        assert_eq!(
            text,
            [
                "A title over both boxes",
                "left 0",
                "left 1",
                "left 2",
                "right 0",
                "right 1",
                "right 2",
                "A line under the boxes"
            ]
        );
        // Without the boxes, the same lines read row by row.
        frags.iter_mut().for_each(|f| f.region = None);
        assert_eq!(lines_of(&frags, true)[1], [1, 2]);
        // Text beside a box of one line stays on its line.
        let mut frags = vec![frag("Total", 50.0, 700.0), frag("10.00", 300.0, 700.0)];
        frags[1].region = Some(0);
        let lines = lines_in(&frags, true, &[[290.0, 690.0, 400.0, 715.0]], &[]);
        assert_eq!(lines, [vec![0, 1]]);
    }

    #[test]
    fn rows_of_two_sizes_are_read_row_by_row() {
        // Labels of 8 points and values of 12, top-aligned: every value sits 2.9 points
        // below its label's baseline, the shift the sizes explain.
        let mut frags = Vec::new();
        for k in 0..6 {
            let top = 700.0 - 20.0 * k as f32;
            frags.push(Frag {
                size: 8.0,
                ..frag(&format!("Label {k}"), 50.0, top - 8.0 * 0.718)
            });
            frags.push(Frag {
                size: 12.0,
                ..frag(&format!("Value {k}"), 200.0, top - 12.0 * 0.718)
            });
        }
        let lines = lines_of(&frags, true);
        assert_eq!(lines.len(), 6, "{lines:?}");
        assert_eq!(lines[0], [0, 1]);
    }

    /// A title of `size` points (bold when `bold`) at x 40 beside field lines of 6 points at
    /// x 360 (label) and x 440 (value) on the baselines `ys`, the title 1.5 points under the
    /// baseline of field line `at`. The fragments: the title first, then label and value of
    /// each line.
    fn title_beside(size: f32, bold: bool, ys: &[f32], at: usize) -> Vec<Frag> {
        title_dropped(size, bold, ys, at, 1.5)
    }

    /// [`title_beside`] with the title `drop` points under the baseline of field line `at`.
    fn title_dropped(size: f32, bold: bool, ys: &[f32], at: usize, drop: f32) -> Vec<Frag> {
        let small = |t: &str, x: f32, y: f32| Frag {
            size: 6.0,
            end: x + 3.0 * t.chars().count() as f32,
            pen: x + 3.0 * t.chars().count() as f32,
            free: x + 3.0 * t.chars().count() as f32,
            ..frag(t, x, y)
        };
        let mut frags = vec![Frag {
            size,
            bold,
            ..frag("Rechnung", 40.0, ys[at] - drop)
        }];
        for (k, &y) in ys.iter().enumerate() {
            frags.push(small(&format!("Feld {k}"), 360.0, y));
            frags.push(small(&format!("Wert {k}"), 440.0, y));
        }
        frags
    }

    /// The lines of [`lines_of`] in columns, as their texts.
    fn line_texts(frags: &[Frag]) -> Vec<String> {
        lines_of(frags, true)
            .iter()
            .map(|l| {
                l.iter()
                    .map(|&i| frags[i].text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }

    #[test]
    fn a_lone_title_beside_a_field_list_is_a_line_of_its_own() {
        let ys = [703.0, 695.0, 687.0, 679.0];
        for (size, bold) in [(9.0, true), (9.0, false), (6.0, true)] {
            for at in 0..ys.len() {
                let lines = line_texts(&title_beside(size, bold, &ys, at));
                let mut want = vec!["Rechnung".to_string()];
                want.extend((0..ys.len()).map(|k| format!("Feld {k} Wert {k}")));
                assert_eq!(lines, want, "{size} pt, bold {bold}, on {at}");
            }
        }
    }

    #[test]
    fn a_lone_title_beside_a_field_list_in_a_box_is_a_line_of_its_own() {
        // The title and the field lines in one drawn box, a line of free text over it: the
        // lines of the box keep the title apart from the field line it shares a row with.
        let mut frags = title_beside(9.0, true, &[703.0, 695.0, 687.0, 679.0], 0);
        frags.iter_mut().for_each(|f| f.region = Some(0));
        frags.push(frag("Muster GmbH", 40.0, 760.0));
        let lines = lines_in(&frags, true, &[[30.0, 670.0, 560.0, 715.0]], &[]);
        let text: Vec<String> = lines
            .iter()
            .map(|l| {
                l.iter()
                    .map(|&i| frags[i].text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        let mut want = vec!["Muster GmbH".to_string(), "Rechnung".to_string()];
        want.extend((0..4).map(|k| format!("Feld {k} Wert {k}")));
        assert_eq!(text, want);
    }

    #[test]
    fn a_lone_cell_on_a_baseline_of_the_block_stays_in_its_row() {
        // On the baseline of a field line, or within 0.15 of the smaller size of it, the
        // larger bold cell is a cell of that line's row.
        let ys = [703.0, 695.0, 687.0, 679.0];
        for drop in [0.0, 0.8, -0.8] {
            for at in 0..ys.len() {
                let lines = line_texts(&title_dropped(9.0, true, &ys, at, drop));
                let want = format!("Rechnung Feld {at} Wert {at}");
                assert_eq!(lines.len(), ys.len(), "{drop}, on {at}: {lines:?}");
                assert_eq!(lines[at], want, "{drop}, on {at}: {lines:?}");
            }
        }
    }

    #[test]
    fn lone_side_needs_one_pitch() {
        // The field lines step 8, 8 and 20 points: no block of one pitch, so no gutter. (The
        // region is then cut at the wide step, and the three lines over it are one.)
        let frags = title_beside(9.0, true, &[703.0, 695.0, 687.0, 667.0], 0);
        let ps = pieces(&frags);
        let ids: Vec<usize> = (0..ps.len()).collect();
        assert_eq!(gutter(&ps, &ids), None);
        // At one pitch, the lone title makes the gutter.
        let frags = title_beside(9.0, true, &[703.0, 695.0, 687.0, 679.0], 0);
        let ps = pieces(&frags);
        let ids: Vec<usize> = (0..ps.len()).collect();
        assert_eq!(gutter(&ps, &ids).map(|g| g.2), Some(Split::Lone));
    }

    #[test]
    fn lone_side_needs_another_size_or_weight() {
        // A plain title of the size of the fields stands in their row: a cell, no title.
        let lines = line_texts(&title_beside(6.0, false, &[703.0, 695.0, 687.0, 679.0], 0));
        assert_eq!(lines[0], "Rechnung Feld 0 Wert 0", "{lines:?}");
        // Sizes differ from 1.08 times on: 6.5 against 6 does, 6.4 does not.
        let lines = line_texts(&title_beside(6.5, false, &[703.0, 695.0, 687.0, 679.0], 0));
        assert_eq!(lines[0], "Rechnung", "{lines:?}");
        let lines = line_texts(&title_beside(6.4, false, &[703.0, 695.0, 687.0, 679.0], 0));
        assert_eq!(lines[0], "Rechnung Feld 0 Wert 0", "{lines:?}");
    }
}
