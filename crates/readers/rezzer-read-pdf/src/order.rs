// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Glyphs to runs in reading order: runs per style and baseline, lines by vertical overlap,
//! columns by white space gutters, right-to-left lines in logical order and vertical writing
//! column by column. Nothing here reads the file; the glyphs come from the text device.

use std::rc::Rc;

use unicode_bidi::{BidiClass, Level, ParagraphBidiInfo, bidi_class};

use crate::layout::Run;
use crate::regions::{BOX_WORK, Index, offset_baselines};
use crate::text::{Gl, Link, Texts};

/// A gap wider than this many font sizes between two glyphs of a line separates two words.
const WORD_GAP: f32 = 0.15;

/// A gap wider than this many font sizes splits a line into pieces, which may stand in
/// different columns.
const PIECE_GAP: f32 = 0.8;

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
    /// Start on the baseline in the frame the run reads along +x.
    x: f32,
    y: f32,
    /// The same start in the page's frame.
    ox: f32,
    oy: f32,
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
    /// Where the first ink starts, and the advance of the white space glyph the run starts
    /// with (0 when it starts with ink).
    ink_x: f32,
    lead: f32,
    size: f32,
    bold: bool,
    italic: bool,
    mcid: Option<u32>,
    /// The URI link over the glyphs, an index into the page's links.
    link: Option<u32>,
    /// The drawn box the run's middle lies in, the innermost one, as an index into the
    /// page's boxes: a reading region.
    region: Option<u32>,
    chars: usize,
    /// Where the text of each glyph (and each space put in) starts in `text`: the units
    /// right-to-left reordering moves as a whole.
    units: Vec<u32>,
}

/// A frame that turns glyph positions so their text reads along +x.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Turn {
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

    /// A point in the turned frame.
    fn apply(self, x: f32, y: f32) -> (f32, f32) {
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
                if gap > WORD_GAP * size && !glued {
                    f.units.push(f.text.len() as u32);
                    f.text.push(' ');
                }
                if !t.is_empty() {
                    f.units.push(f.text.len() as u32);
                }
                f.text.push_str(t);
                f.chars += t.chars().count();
                f.pen = f.pen.max(x + adv);
                if pad {
                    if !f.padded {
                        f.free = f.free.max(x + adv);
                        f.padded = true;
                    }
                } else {
                    if !f.inked {
                        f.ink_x = x;
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
            end: if pad { x } else { x + adv },
            pen: x + adv,
            inked: !pad,
            free: x + adv,
            padded: pad,
            ink_x: x,
            lead: if pad { adv } else { 0.0 },
            size,
            bold: g.bold,
            italic: g.italic,
            mcid: g.mcid,
            link: g.link,
            region: None,
            chars: t.chars().count(),
            units: vec![0],
        });
    }
    out
}

/// A row of fragments on one baseline, split where a gap is wide enough to be a column
/// gutter.
#[derive(Debug, Clone)]
struct Piece {
    row: usize,
    x0: f32,
    /// The end of the piece's last glyph, white space too.
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
    chars: usize,
    frags: Vec<usize>,
    /// The piece stands in for the text of a drawn box (the index of the box's group), read
    /// whole where the piece is read; its rows are counted in `rows`.
    stand: Option<usize>,
    rows: usize,
}

/// The white space between a run or piece whose gap counts from `free` (`padded` when white
/// space ends it) and the first ink of `f`, less one space: a gap of padding, not of a word
/// space.
fn gap(free: f32, padded: bool, f: &Frag) -> f32 {
    f.ink_x - free - if padded { 0.0 } else { f.lead }
}

/// The vertical extent of a fragment: a quarter of its size below the baseline, three
/// quarters above.
fn extent(f: &Frag) -> (f32, f32) {
    (f.y - 0.25 * f.size, f.y + 0.75 * f.size)
}

/// For each fragment, the first of the chain it goes on in content order: a fragment that
/// goes on right where the one before it ends, on its baseline in its size and in its box (a
/// run cut at a link's edge or a change of style), moves with it, so other text that shares
/// the row is never put in between.
fn chain_heads(frags: &[Frag]) -> Vec<usize> {
    let mut head: Vec<usize> = Vec::with_capacity(frags.len());
    for (i, f) in frags.iter().enumerate() {
        let joins = i > 0 && {
            let p = &frags[i - 1];
            let size = p.size.max(f.size);
            p.region == f.region
                && (f.y - p.y).abs() <= 0.1 * size
                && (f.size - p.size).abs() <= 0.05 * size
                && (f.x - p.pen).abs() <= 0.1 * size
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
                    p.x1 = p.x1.max(f.pen);
                    if f.inked {
                        (p.free, p.padded) = (f.free, f.padded);
                    } else if !p.padded {
                        (p.free, p.padded) = (f.free, true);
                    }
                    p.y0 = p.y0.min(lo);
                    p.y1 = p.y1.max(hi);
                    p.size = p.size.max(f.size);
                    p.chars += f.chars;
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
            });
        }
        out.extend(piece);
    }
    (out, count)
}

/// The pieces in reading order: columns one after the other, with the pieces that cross the
/// gutter (titles, notes across the columns) splitting the columns into bands.
fn reading_order(pieces: &[Piece]) -> Vec<usize> {
    let mut out = Vec::with_capacity(pieces.len());
    region(pieces, (0..pieces.len()).collect(), 0, &mut out);
    out
}

fn region(p: &[Piece], mut ids: Vec<usize>, depth: usize, out: &mut Vec<usize>) {
    let deep = ids.len() > 3 && depth < COLUMN_DEPTH && ids.len() <= COLUMN_PIECES;
    let found = if deep { gutter(p, &ids) } else { None };
    // A gutter that only a drawn box makes is taken after the bands of white space across:
    // text above or below the boxes is not part of their columns.
    if let Some((_, _, true)) = found {
        ids.sort_by(|&a, &b| p[a].row.cmp(&p[b].row).then(p[a].x0.total_cmp(&p[b].x0)));
        if let Some(at) = widest_gap(p, &ids) {
            let rest = ids.split_off(at);
            region(p, ids, depth + 1, out);
            region(p, rest, depth + 1, out);
            return;
        }
    }
    if let Some((g0, g1, _)) = found {
        ids.sort_by(|&a, &b| p[a].row.cmp(&p[b].row).then(p[a].x0.total_cmp(&p[b].x0)));
        let mut band: Vec<usize> = Vec::new();
        let flush = |band: &mut Vec<usize>, out: &mut Vec<usize>| {
            if band.is_empty() {
                return;
            }
            let (left, right): (Vec<usize>, Vec<usize>) =
                band.drain(..).partition(|&i| p[i].x1 <= g0 + 0.5);
            region(p, left, depth + 1, out);
            region(p, right, depth + 1, out);
        };
        for id in ids {
            if p[id].x1 > g0 + 0.5 && p[id].x0 < g1 - 0.5 {
                flush(&mut band, out);
                out.push(id);
            } else {
                band.push(id);
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
        region(p, ids, depth + 1, out);
        region(p, rest, depth + 1, out);
        return;
    }
    out.extend(ids);
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

/// A column gutter in a region: the widest vertical band of white space that only a few
/// pieces cross, with blocks on both sides that stand side by side: prose, lines whose
/// baselines do not line up, or a drawn box. The flag says that only a box makes it.
fn gutter(p: &[Piece], ids: &[usize]) -> Option<(f32, f32, bool)> {
    let lo = ids.iter().map(|&i| p[i].x0).fold(f32::MAX, f32::min);
    let hi = ids.iter().map(|&i| p[i].x1).fold(f32::MIN, f32::max);
    if !(hi - lo).is_finite() || hi - lo < 2.0 {
        return None;
    }
    let width = ((hi - lo).ceil() as usize).min(20_000) + 2;
    let scale = (hi - lo) / (width - 2) as f32;
    let bin = |x: f32| (((x - lo) / scale) as usize).min(width - 1);
    let mut diff = vec![0i32; width + 1];
    for &i in ids {
        diff[bin(p[i].x0)] += 1;
        diff[bin(p[i].x1) + 1] -= 1;
    }
    let allowed = (CROSSERS * ids.len() as f32).floor() as i32;
    let mut sizes: Vec<f32> = ids.iter().map(|&i| p[i].size).collect();
    sizes.sort_by(f32::total_cmp);
    let min_gap = GUTTER * sizes[sizes.len() / 2];
    let mut cover = Vec::with_capacity(width);
    let mut c = 0;
    for d in diff.iter().take(width) {
        c += d;
        cover.push(c);
    }
    // The band covered by the fewest pieces wins: a ragged line end next to the gutter must
    // not widen it. Only bands with text on both sides count.
    let band = |level: i32| {
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
        best
    };
    let best = (0..=allowed).find_map(band);
    let (g0, g1) = best?;
    let crossing = ids
        .iter()
        .filter(|&&i| p[i].x1 > g0 + 0.5 && p[i].x0 < g1 - 0.5)
        .count();
    if crossing as i32 > allowed {
        return None;
    }
    let left: Vec<usize> = ids
        .iter()
        .copied()
        .filter(|&i| p[i].x1 <= g0 + 0.5)
        .collect();
    let right: Vec<usize> = ids
        .iter()
        .copied()
        .filter(|&i| p[i].x0 >= g1 - 0.5)
        .collect();
    let prose = |side: &[usize]| {
        if side.len() < 2 {
            return false;
        }
        let mut chars: Vec<usize> = side.iter().map(|&i| p[i].chars).collect();
        chars.sort_unstable();
        chars[chars.len() / 2] >= PROSE_CHARS
    };
    // A side that holds a drawn box of two lines or more is a block of its own.
    let boxed = |side: &[usize]| side.iter().any(|&i| p[i].stand.is_some() && p[i].rows >= 2);
    // The baselines of a side, one for each of its rows.
    // The baselines of a side, line by line: those of the pieces of each row.
    let baselines = |side: &[usize]| {
        let mut rows: Vec<(usize, f32)> = side.iter().map(|&i| (p[i].row, p[i].y)).collect();
        rows.sort_by_key(|r| r.0);
        let mut out: Vec<Vec<f32>> = Vec::new();
        let mut last = None;
        for (row, y) in rows {
            match out.last_mut() {
                Some(l) if last == Some(row) => l.push(y),
                _ => out.push(vec![y]),
            }
            last = Some(row);
        }
        out
    };
    let size = sizes[sizes.len() / 2];
    let apart = || offset_baselines(&baselines(&left), &baselines(&right), size);
    let text = (prose(&left) && prose(&right)) || apart();
    let by_box = !text && !left.is_empty() && !right.is_empty() && (boxed(&left) || boxed(&right));
    if !text && !by_box {
        return None;
    }
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
    (overlap / smaller >= 0.3).then_some((g0, g1, by_box))
}

/// Lines of fragments in reading order, as lists of fragment indices.
#[cfg(test)]
fn lines_of(frags: &[Frag], columns: bool) -> Vec<Vec<usize>> {
    lines_in(frags, columns, &[])
}

/// Lines of fragments in reading order, as lists of fragment indices. A fragment tagged with
/// one of `boxes` (in the frame of the fragments) is read with the rest of its box: the box
/// stands as one piece among the others, ordered as they are, and its text is read whole
/// where it stands.
fn lines_in(frags: &[Frag], columns: bool, boxes: &[[f32; 4]]) -> Vec<Vec<usize>> {
    let head = chain_heads(frags);
    let boxed = columns && frags.iter().any(|f| f.region.is_some());
    let order: Vec<usize>;
    let ps: Vec<Piece>;
    if !boxed {
        let all: Vec<usize> = (0..frags.len()).collect();
        ps = pieces_in(frags, &all, &head, 0).0;
        order = if columns {
            reading_order(&ps)
        } else {
            let mut o: Vec<usize> = (0..ps.len()).collect();
            o.sort_by(|&a, &b| {
                ps[a]
                    .row
                    .cmp(&ps[b].row)
                    .then(ps[a].x0.total_cmp(&ps[b].x0))
            });
            o
        };
    } else {
        (ps, order) = boxed_order(frags, &head, boxes);
    }
    let mut lines: Vec<Vec<usize>> = Vec::new();
    let mut last_row = None;
    for i in order {
        let p = &ps[i];
        if last_row == Some(p.row)
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
fn boxed_order(frags: &[Frag], head: &[usize], boxes: &[[f32; 4]]) -> (Vec<Piece>, Vec<usize>) {
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
    let mut out = Vec::with_capacity(ps.len());
    region(&ps, top, 0, &mut out);
    let mut order = Vec::with_capacity(ps.len());
    for id in out {
        match ps[id].stand {
            Some(g) => region(&ps, inner[g].clone().collect(), 0, &mut order),
            None => order.push(id),
        }
    }
    (ps, order)
}

/// The runs of one line, numbered `line`. A run that starts a new word gets a leading space;
/// a line with right-to-left text becomes one run in logical order. A run's link is the one
/// of `links` its glyphs lie in.
fn line_runs(page: u32, frags: &[Frag], ids: &[usize], line: u32, links: &[Link]) -> Vec<Run> {
    let n = ids.len();
    // For each fragment: the part of its text that is kept, whether its padding is written as
    // one space after it, and whether a space is put in before it.
    let mut keep: Vec<(usize, usize)> = ids.iter().map(|&i| (0, frags[i].text.len())).collect();
    let mut pad_after = vec![false; n];
    let mut spaced = vec![false; n];
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
        if !glued && f.x - p.end > WORD_GAP * f.size.max(p.size) {
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
        mcid: f.mcid,
        uri: uri(f),
        line,
        off_axis: false,
    };
    let rtl = ids.iter().any(|&i| frags[i].text.chars().any(is_rtl));
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
                run_of(f, text)
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
                mcid: g.mcid,
                uri: None,
                line: 0,
                off_axis: true,
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
    runs_in(page, glyphs, text, turn, links, &[]).0
}

/// [`runs_of`], with the drawn boxes of the page (in the frame of the page box) as reading
/// regions for the main text: the text of a box is read whole. Also says why the boxes were
/// not used, when a bound kept them out.
pub(crate) fn runs_in(
    page: u32,
    glyphs: &[Gl],
    text: &Texts<'_>,
    turn: Turn,
    links: &[Link],
    boxes: &[[f32; 4]],
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
    let unused = if turned.is_empty() {
        None
    } else {
        tag_regions(&mut frags, &turned).err()
    };
    for ids in lines_in(&frags, true, &turned) {
        runs.extend(line_runs(page, &frags, &ids, line, links));
        line += 1;
    }
    for (dir, list) in others {
        // Lines are found in the frame where this text reads along +x; the runs keep the
        // position of their first glyph in the page's frame.
        let frame = compose(turn, dir);
        let frags = fragments(&list, text, frame, turn);
        for ids in lines_in(&frags, false, &[]) {
            let mut r = line_runs(page, &frags, &ids, line, links);
            for (run, &i) in r.iter_mut().zip(&ids) {
                (run.x, run.y) = (frags[i].ox, frags[i].oy);
                run.off_axis = true;
            }
            runs.extend(r);
            line += 1;
        }
    }
    runs.extend(vertical_runs(page, &vertical, text, turn, line));
    (runs, unused)
}

/// Tags each fragment with the innermost of `boxes` its middle lies in. A box that holds
/// another box with text is no region: the inner one is. Past the bounds no fragment is
/// tagged, and the reason is given.
fn tag_regions(frags: &mut [Frag], boxes: &[[f32; 4]]) -> Result<(), String> {
    let cut = |frags: &mut [Frag], why: &str| {
        frags.iter_mut().for_each(|f| f.region = None);
        Err(why.to_string())
    };
    let Some(mut index) = Index::new(boxes) else {
        return cut(frags, "its boxes overlap too much");
    };
    let mut held = vec![false; boxes.len()];
    for f in frags.iter_mut() {
        let (x, y) = ((f.x + f.pen.max(f.x)) / 2.0, f.y + 0.25 * f.size);
        f.region = index.innermost(x, y);
        if let Some(b) = f.region {
            held[b as usize] = true;
        }
        if index.work > BOX_WORK {
            return cut(frags, "its boxes take too many steps to look up");
        }
    }
    // Boxes that hold a box with text are outer ones.
    let mut outer = vec![false; boxes.len()];
    for (k, b) in boxes.iter().enumerate() {
        if !held[k] {
            continue;
        }
        index.holders((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0, |o| {
            let a = boxes[o as usize];
            if o as usize != k && a[0] <= b[0] && a[1] <= b[1] && a[2] >= b[2] && a[3] >= b[3] {
                outer[o as usize] = true;
            }
        });
        if index.work > BOX_WORK {
            return cut(frags, "its boxes take too many steps to look up");
        }
    }
    for f in frags.iter_mut() {
        if f.region.is_some_and(|b| outer[b as usize]) {
            f.region = None;
        }
    }
    Ok(())
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
            end: x + 6.0 * text.chars().count() as f32,
            pen: x + 6.0 * text.chars().count() as f32,
            inked: true,
            free: x + 6.0 * text.chars().count() as f32,
            padded: false,
            ink_x: x,
            lead: 0.0,
            size: 10.0,
            bold: false,
            italic: false,
            mcid: None,
            link: None,
            region: None,
            chars: text.chars().count(),
            units: vec![0],
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
        let lines = lines_in(&frags, true, &boxes);
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
        let lines = lines_in(&frags, true, &[[290.0, 690.0, 400.0, 715.0]]);
        assert_eq!(lines, [vec![0, 1]]);
    }
}
