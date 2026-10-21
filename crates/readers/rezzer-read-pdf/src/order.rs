// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Glyphs to runs in reading order: runs per style and baseline, lines by vertical overlap,
//! columns by white space gutters, right-to-left lines in logical order and vertical writing
//! column by column. Nothing here reads the file; the glyphs come from the text device.

use unicode_bidi::{BidiClass, Level, ParagraphBidiInfo, bidi_class};

use crate::layout::Run;
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

/// White space that pads text: a space, a no-break space and the typographic spaces.
fn is_pad(c: char) -> bool {
    matches!(c, ' ' | '\u{a0}' | '\u{2000}'..='\u{200a}')
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
    size: f32,
    bold: bool,
    italic: bool,
    mcid: Option<u32>,
    /// The URI link over the glyphs, an index into the page's links.
    link: Option<u32>,
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
            let gap = x - if pad || !f.inked { f.pen } else { f.end };
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
                if !pad {
                    f.end = if f.inked { f.end.max(x + adv) } else { x + adv };
                    f.inked = true;
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
            size,
            bold: g.bold,
            italic: g.italic,
            mcid: g.mcid,
            link: g.link,
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
    /// The end of its last glyph with ink, where the gap to the next fragment is measured.
    ink: f32,
    y0: f32,
    y1: f32,
    size: f32,
    chars: usize,
    frags: Vec<usize>,
}

/// The vertical extent of a fragment: a quarter of its size below the baseline, three
/// quarters above.
fn extent(f: &Frag) -> (f32, f32) {
    (f.y - 0.25 * f.size, f.y + 0.75 * f.size)
}

/// Rows of fragments that share a baseline (by vertical overlap), top to bottom, each split
/// into pieces at wide gaps. Fragments drawn twice in one place are dropped.
fn pieces(frags: &[Frag]) -> Vec<Piece> {
    let mut order: Vec<usize> = (0..frags.len()).collect();
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
    // A fragment that goes on right where the one before it in content order ends, on its
    // baseline in its size (a run cut at a link's edge or a change of style), moves with it:
    // other text that shares the row is never put in between.
    let mut head: Vec<usize> = Vec::with_capacity(frags.len());
    for (i, f) in frags.iter().enumerate() {
        let joins = i > 0 && {
            let p = &frags[i - 1];
            let size = p.size.max(f.size);
            (f.y - p.y).abs() <= 0.1 * size
                && (f.size - p.size).abs() <= 0.05 * size
                && (f.x - p.pen).abs() <= 0.1 * size
        };
        head.push(if joins { head[i - 1] } else { i });
    }
    let mut out = Vec::new();
    for (r, mut row) in rows.into_iter().enumerate() {
        row.sort_by(|&a, &b| {
            frags[head[a]]
                .x
                .total_cmp(&frags[head[b]].x)
                .then(head[a].cmp(&head[b]))
                .then(a.cmp(&b))
        });
        // The same text drawn again on top of itself (a fake bold) is read once.
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
        let mut piece: Option<Piece> = None;
        for i in kept {
            let f = &frags[i];
            let (lo, hi) = extent(f);
            if let Some(p) = piece.as_mut() {
                if f.x - p.ink <= PIECE_GAP * f.size.max(p.size) {
                    p.x1 = p.x1.max(f.pen);
                    p.ink = p.ink.max(f.end);
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
                row: r,
                x0: f.x,
                x1: f.pen.max(f.x),
                ink: f.end.max(f.x),
                y0: lo,
                y1: hi,
                size: f.size,
                chars: f.chars,
                frags: vec![i],
            });
        }
        out.extend(piece);
    }
    out
}

/// The pieces in reading order: columns one after the other, with the pieces that cross the
/// gutter (titles, notes across the columns) splitting the columns into bands.
fn reading_order(pieces: &[Piece]) -> Vec<usize> {
    let mut out = Vec::with_capacity(pieces.len());
    region(pieces, (0..pieces.len()).collect(), 0, &mut out);
    out
}

fn region(p: &[Piece], mut ids: Vec<usize>, depth: usize, out: &mut Vec<usize>) {
    if ids.len() > 3
        && depth < COLUMN_DEPTH
        && ids.len() <= COLUMN_PIECES
        && let Some((g0, g1)) = gutter(p, &ids)
    {
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
/// pieces cross, with prose columns on both sides that stand side by side.
fn gutter(p: &[Piece], ids: &[usize]) -> Option<(f32, f32)> {
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
    if !prose(&left) || !prose(&right) {
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
    (overlap / smaller >= 0.3).then_some((g0, g1))
}

/// Lines of fragments in reading order, as lists of fragment indices.
fn lines_of(frags: &[Frag], columns: bool) -> Vec<Vec<usize>> {
    let ps = pieces(frags);
    let order = if columns {
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

/// The runs of one line, numbered `line`. A run that starts a new word gets a leading space;
/// a line with right-to-left text becomes one run in logical order. A run's link is the one
/// of `links` its glyphs lie in.
fn line_runs(page: u32, frags: &[Frag], ids: &[usize], line: u32, links: &[Link]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::with_capacity(ids.len());
    let mut prev: Option<&Frag> = None;
    // Whether each run got a space put in before it.
    let mut spaced: Vec<bool> = Vec::with_capacity(ids.len());
    for &i in ids {
        let f = &frags[i];
        let mut text = collapsed(&f.text);
        spaced.push(false);
        if let Some(p) = prev {
            let gap = f.x - p.end;
            let glued =
                p.text.ends_with(char::is_whitespace) || text.starts_with(char::is_whitespace);
            if !glued && gap > WORD_GAP * f.size.max(p.size) {
                text.insert(0, ' ');
                if let Some(s) = spaced.last_mut() {
                    *s = true;
                }
            }
        }
        runs.push(Run {
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
            uri: f
                .link
                .and_then(|k| links.get(k as usize))
                .map(|(_, u)| u.clone()),
            line,
            off_axis: false,
        });
        prev = Some(f);
    }
    if runs.iter().any(|r| r.text.chars().any(is_rtl)) {
        // The glyphs stand in visual order. Reordering a visual line by the bidi rules gives
        // the logical order back; the text of one glyph (a ligature mapped to several
        // characters) moves as a whole.
        let mut units: Vec<&str> = Vec::new();
        for (&spaced, &i) in spaced.iter().zip(ids) {
            let f = &frags[i];
            if spaced {
                units.push(" ");
            }
            let mut starts: Vec<usize> = f.units.iter().map(|&u| u as usize).collect();
            starts.push(f.text.len());
            for w in starts.windows(2) {
                if w[1] > w[0] {
                    units.push(&f.text[w[0]..w[1]]);
                }
            }
        }
        let logical = logical_order(&units);
        let mut one = runs.swap_remove(0);
        let x0 = runs.iter().map(|r| r.x).fold(one.x, f32::min);
        let x1 = runs.iter().map(|r| r.x + r.w).fold(one.x + one.w, f32::max);
        one.text = logical;
        one.x = x0;
        one.w = x1 - x0;
        runs = vec![one];
    }
    runs
}

/// `text` with each stretch of two or more padding spaces made one space.
fn collapsed(text: &str) -> String {
    let mut chars = text.chars().peekable();
    let mut doubled = false;
    while let Some(c) = chars.next() {
        if is_pad(c) && chars.peek().is_some_and(|&n| is_pad(n)) {
            doubled = true;
            break;
        }
    }
    if !doubled {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut stretch = 0usize;
    let mut first = ' ';
    for c in text.chars() {
        if is_pad(c) {
            if stretch == 0 {
                first = c;
            }
            stretch += 1;
            continue;
        }
        match stretch {
            0 => {}
            1 => out.push(first),
            _ => out.push(' '),
        }
        stretch = 0;
        out.push(c);
    }
    match stretch {
        0 => {}
        1 => out.push(first),
        _ => out.push(' '),
    }
    out
}

/// The units of a visual line in logical order. In the bidi algorithm each unit stands for
/// its first strongly directed character; a unit with text of both directions is already in
/// logical order inside and stands for a space, taking the direction around it.
fn logical_order(units: &[&str]) -> String {
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
        .filter_map(|i| units.get(i).copied())
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
    let frags = fragments(&main, text, turn, turn);
    for ids in lines_of(&frags, true) {
        runs.extend(line_runs(page, &frags, &ids, line, links));
        line += 1;
    }
    for (dir, list) in others {
        // Lines are found in the frame where this text reads along +x; the runs keep the
        // position of their first glyph in the page's frame.
        let frame = compose(turn, dir);
        let frags = fragments(&list, text, frame, turn);
        for ids in lines_of(&frags, false) {
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
    runs
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
            size: 10.0,
            bold: false,
            italic: false,
            mcid: None,
            link: None,
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
    fn padding_splits_pieces_but_not_words() {
        // A label, white space padding past the piece gap, a value: two pieces. A word gap
        // of one space keeps one piece.
        let pad = |text: &str, x: f32| Frag {
            end: x,
            inked: false,
            ..frag(text, x, 700.0)
        };
        let mut label = frag("Points:", 50.0, 700.0);
        label.pen = 50.0 + 6.0 * 7.0 + 30.0;
        label.text.push_str("\u{a0}\u{a0}\u{a0}\u{a0}\u{a0}");
        let value = frag("1200", 50.0 + 6.0 * 7.0 + 30.0, 700.0);
        let ps = pieces(&[label, value]);
        assert_eq!(ps.len(), 2, "{ps:?}");
        // The piece of the label reaches over its padding.
        assert_eq!(ps[0].x1, 122.0);
        let ps = pieces(&[pad(" ", 40.0), frag("word", 43.0, 700.0)]);
        assert_eq!(ps.len(), 1, "{ps:?}");
        assert_eq!(collapsed("a\u{a0}\u{a0}\u{a0}b c\u{a0}d"), "a b c\u{a0}d");
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
}
