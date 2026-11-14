// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The spatial reading of a region whose kind is not certain: its text as it stands, line by
//! line, each line in pieces at their columns, so a person or an agent can still make sense of
//! it. It keeps every text run of the region exactly once, in reading order.

use std::collections::BTreeMap;

use crate::extract::pairs::blocks::PITCH_ROUNDING;
use crate::input::{PageIn, PrimId, TextRun};

/// One line of a spatial reading: its pieces left to right, each with its column (in
/// character advances from the region's left edge) and its runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpatialLineOut {
    pub pieces: Vec<(u32 /* col */, Vec<PrimId>)>,
}

/// A gap of this many character advances or more between two runs of a line starts a new
/// piece.
pub const SPATIAL_PIECE_GAP: f32 = 2.0;

/// The spatial reading of the text runs `prims` of `page`: one line per line number, top to
/// bottom in the reader's line order; in a line its runs left to right (by left edge, then by
/// id), in pieces where the gap from the right end of the piece so far to the next run's left
/// edge is [`SPATIAL_PIECE_GAP`] advances or more (runs that overlap stay in their piece). A
/// piece's column is `((x0 - region x0) / adv).round()`, `adv` the median advance per character
/// of the region's runs, `region x0` the least left edge of its runs.
///
/// Every text run of `prims` is in exactly one piece, once: runs of white space and runs off
/// the line's axis too (in the line of their number). Primitives other than text are no lines
/// and are not in the reading: the caller shows them by their own kind (a picture as needing
/// vision, a code with its value). Ids the page does not have are skipped (a debug build
/// asserts on them). Runs without width or characters give no advance: then `adv` is half the
/// median text size, or 1 without any size, so no column is undefined.
pub fn spatial(page: &PageIn, prims: &[PrimId]) -> Vec<SpatialLineOut> {
    let mut ids: Vec<u32> = prims
        .iter()
        .filter_map(|p| match *p {
            PrimId::Text(i) if (i as usize) < page.texts.len() => Some(i),
            PrimId::Text(i) => {
                debug_assert!(false, "run {i} is not on the page");
                None
            }
            _ => None,
        })
        .collect();
    ids.sort_unstable();
    ids.dedup();
    crate::work::add(1 + ids.len() as u64);
    if ids.is_empty() {
        return Vec::new();
    }
    let run = |i: u32| -> &TextRun { &page.texts[i as usize] };
    let left = |r: &TextRun| r.x.min(r.x + r.w);
    let right = |r: &TextRun| r.x.max(r.x + r.w);
    let (x0, adv) = origin(ids.iter().map(|&i| run(i)));
    ids.sort_by(|&a, &b| {
        let (ra, rb) = (run(a), run(b));
        ra.line
            .cmp(&rb.line)
            .then(left(ra).total_cmp(&left(rb)))
            .then(a.cmp(&b))
    });
    let col = |x: f32| -> u32 {
        let c = ((x - x0) / adv).round();
        if c.is_finite() && c > 0.0 {
            c.min(u32::MAX as f32) as u32
        } else {
            0
        }
    };
    let mut out = Vec::new();
    for line in ids.chunk_by(|&a, &b| run(a).line == run(b).line) {
        let mut pieces: Vec<(u32, Vec<PrimId>)> = Vec::new();
        let mut end = f32::MIN;
        for &i in line {
            let r = run(i);
            match pieces.last_mut() {
                Some((_, p)) if left(r) - end < SPATIAL_PIECE_GAP * adv => p.push(PrimId::Text(i)),
                _ => pieces.push((col(left(r)), vec![PrimId::Text(i)])),
            }
            end = end.max(right(r));
        }
        out.push(SpatialLineOut { pieces });
    }
    out
}

/// The left edge the columns of a spatial reading are counted from (the least left edge of
/// `runs`) and the width of one column (their [`advance`]).
fn origin<'a>(runs: impl Iterator<Item = &'a TextRun> + Clone) -> (f32, f32) {
    let x0 = runs
        .clone()
        .map(|r| r.x.min(r.x + r.w))
        .fold(f32::MAX, f32::min);
    (x0, advance(runs))
}

/// Where a spatial reading stands on its page: what [`spatial`] counts its columns from and
/// in, and the baselines of its lines. Two readings are told to be rows of one small table
/// by it ([`SpatialFrame::under`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpatialFrame {
    /// The left edge column 0 stands at.
    pub x0: f32,
    /// The width of one column.
    pub adv: f32,
    /// The baseline of its first line.
    pub first: f32,
    /// The baseline of its last line.
    pub last: f32,
    /// Its text size (the median of its runs), 0 when no run has one.
    pub size: f32,
    /// The pitch of its lines: the median step between their baselines, the lower one of
    /// two. A reading of one line has none.
    pub pitch: Option<f32>,
}

/// The frame of the spatial reading of the text runs `prims` of `page` (see [`spatial`]:
/// the same runs, the same left edge and advance). None without a text run.
pub fn frame(page: &PageIn, prims: &[PrimId]) -> Option<SpatialFrame> {
    let mut runs: Vec<&TextRun> = prims
        .iter()
        .filter_map(|p| match *p {
            PrimId::Text(i) => page.texts.get(i as usize),
            _ => None,
        })
        .collect();
    crate::work::add(1 + runs.len() as u64);
    if runs.is_empty() {
        return None;
    }
    let (x0, adv) = origin(runs.iter().copied());
    // The baseline of a line: that of its leftmost run on the baseline with ink, else of
    // its leftmost run.
    let rank = |r: &TextRun| {
        (
            r.line,
            r.shift != crate::input::Shift::Base || r.text.trim().is_empty(),
        )
    };
    runs.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then(a.x.min(a.x + a.w).total_cmp(&b.x.min(b.x + b.w)))
    });
    let mut lines: Vec<f32> = Vec::new();
    let mut last_line = None;
    for r in &runs {
        if last_line != Some(r.line) {
            lines.push(r.y);
            last_line = Some(r.line);
        }
    }
    let mut steps: Vec<f32> = lines.windows(2).map(|w| (w[0] - w[1]).abs()).collect();
    steps.sort_by(f32::total_cmp);
    let pitch = (!steps.is_empty()).then(|| steps[(steps.len() - 1) / 2]);
    let mut sizes: Vec<f32> = runs
        .iter()
        .map(|r| r.size)
        .filter(|s| s.is_finite() && *s > 0.0)
        .collect();
    sizes.sort_by(f32::total_cmp);
    Some(SpatialFrame {
        x0,
        adv,
        first: *lines.first()?,
        last: *lines.last()?,
        size: sizes.get(sizes.len() / 2).copied().unwrap_or(0.0),
        pitch,
    })
}

/// The grid of a page, in the page's own text: what the spatial readings of a page are
/// compared on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageGrid {
    /// The left edge of the page's text: column 0.
    pub left: f32,
    /// The width of one column: the median advance per character of the page's text runs.
    pub unit: f32,
    /// The line pitch the page shows ([`line_pitch`]), when it shows one.
    pub pitch: Option<f32>,
}

/// The grid of `page`. None for a page without text.
pub fn page_grid(page: &PageIn) -> Option<PageGrid> {
    crate::work::add(1 + page.texts.len() as u64);
    if page.texts.is_empty() {
        return None;
    }
    let (left, unit) = origin(page.texts.iter());
    Some(PageGrid {
        left,
        unit,
        pitch: line_pitch(page),
    })
}

/// A page shows a line pitch when this many steps between its rows or more are that step:
/// one step alone is no pitch, and two equal steps are three lines, which a page of a few
/// lone lines shows by chance. With three, four rows stand at one pitch.
pub const PITCH_STEPS: usize = 3;

/// The line pitch `page` shows: the median step between the baselines of its rows (the
/// lower one of two), when more than half of the steps, and at least [`PITCH_STEPS`], are
/// that step. The rows are the baselines of the runs with ink on the line's axis and on the
/// baseline; baselines within [`ROW_BASELINE`] of the page's text size (the median of those
/// runs) are one row, and steps that differ by no more are one step. A listing set on the
/// lines of a printer shows its pitch this way (text of 7 points every 12 points); a page
/// of lone lines, or of columns whose lines do not meet, shows none.
pub fn line_pitch(page: &PageIn) -> Option<f32> {
    let inked = || {
        page.texts.iter().filter(|r| {
            !r.off_axis && r.shift == crate::input::Shift::Base && !r.text.trim().is_empty()
        })
    };
    let mut sizes: Vec<f32> = inked()
        .map(|r| r.size)
        .filter(|s| s.is_finite() && *s > 0.0)
        .collect();
    sizes.sort_by(f32::total_cmp);
    let near = ROW_BASELINE * sizes.get(sizes.len() / 2).copied()?;
    let mut ys: Vec<f32> = inked().map(|r| r.y).filter(|y| y.is_finite()).collect();
    crate::work::add(1 + 2 * ys.len() as u64);
    ys.sort_by(f32::total_cmp);
    // One baseline for each row: the first of those within `near` of it.
    ys.dedup_by(|y, row| *y - *row <= near);
    let mut steps: Vec<f32> = ys.windows(2).map(|w| w[1] - w[0]).collect();
    if steps.len() < PITCH_STEPS {
        return None;
    }
    steps.sort_by(f32::total_cmp);
    let pitch = steps[(steps.len() - 1) / 2];
    let alike = steps.iter().filter(|s| (*s - pitch).abs() <= near).count();
    (alike >= PITCH_STEPS && 2 * alike > steps.len()).then_some(pitch)
}

/// Two spatial readings start in the same column of the page grid when their left edges
/// are less than this many columns of it apart: a column is one character wide, so edges
/// closer than that can stand in one column, and edges a column or more apart never do. In
/// columns of the page's text, so it holds at every scale.
pub const SAME_COLUMN: f32 = 1.0;

/// Rows of one small table follow each other at most this many line pitches apart,
/// baseline to baseline: a row right under the other, or one empty row between them
/// (a row of labels, a blank line, the row of values). Further down a line is not a row of
/// the lines over it: it belongs to what stands around it there.
pub const RUN_PITCHES: f32 = 2.0;

/// The pitch of a reading of one line, which has none of its own, in text sizes: what one
/// line of text steps down by. Where the page shows a wider line pitch ([`line_pitch`]),
/// that is the pitch of the line.
pub const RUN_LEADING: f32 = crate::extract::text::LEADING;

impl SpatialFrame {
    /// Whether this reading stands right under `above` as a further row in the columns of a
    /// run of readings whose first starts at the left edge `edge`: its left edge is in the
    /// same column of the grid of the page ([`SAME_COLUMN`] columns of `page`), and its
    /// first baseline is under the last one of `above` (more than [`ROW_BASELINE`] sizes, so
    /// it is no piece of the same row) by at most [`RUN_PITCHES`] line pitches (with the
    /// slack of a pitch read from two baselines, `PITCH_ROUNDING` of the size). The pitch is
    /// the larger one of the two readings: each its own, or for one line the pitch the page
    /// shows, at least [`RUN_LEADING`] of its text size. The size is the larger of the two.
    /// Positive evidence only: without a size or a column width it is false.
    pub fn under(&self, above: &SpatialFrame, edge: f32, page: &PageGrid) -> bool {
        let size = self.size.max(above.size);
        let unit = page.unit;
        if !(size > 0.0 && unit > 0.0 && unit.is_finite()) {
            return false;
        }
        if (self.x0 - edge).abs() >= SAME_COLUMN * unit {
            return false;
        }
        let shown = page.pitch.filter(|p| p.is_finite()).unwrap_or(0.0);
        let own = |f: &SpatialFrame| f.pitch.unwrap_or((RUN_LEADING * f.size).max(shown));
        let pitch = own(self).max(own(above));
        let gap = above.last - self.first;
        gap > ROW_BASELINE * size && gap <= RUN_PITCHES * pitch + PITCH_ROUNDING * size
    }

    /// The column of this reading's column `col` on a grid that starts at the left edge
    /// `x0` with columns `adv` wide: the grid of a run of readings (the least left edge and
    /// the least column width among them, so no reading has less room on it than on its
    /// own), on which the pieces of all of them stand in common columns. A place left of
    /// `x0` is column 0.
    pub fn column_on(&self, col: u32, x0: f32, adv: f32) -> u32 {
        let x = self.x0 + col as f32 * self.adv;
        let c = ((x - x0) / adv).round();
        if c.is_finite() && c > 0.0 {
            c.min(u32::MAX as f32) as u32
        } else {
            0
        }
    }
}

/// Two regions share a row when their runs on it stand on baselines at most this many text
/// sizes apart.
pub const ROW_BASELINE: f32 = 0.25;

/// Regions read row by row together share at least this many rows.
pub const MIN_SHARED_ROWS: usize = 2;

/// Whether the text runs `other` (a neighbouring text region) stand on the rows of the text
/// runs `unsure` (a region whose kind is not certain), side by side across a gap, so the two
/// are read row by row as one spatial reading and each line of one stays beside its line of the
/// other (a column of lone amounts beside the column of the names they belong to).
///
/// Positive evidence only: both have runs with ink, none off the line's axis; their rows (line
/// numbers of runs with ink) are the same rows, at least [`MIN_SHARED_ROWS`]; on each row the
/// first runs of the two stand within [`ROW_BASELINE`] text sizes of each other; and the ink of
/// one ends left of the ink of the other with a gap of at least [`SPATIAL_PIECE_GAP`] advances
/// (the median advance of both, as [`spatial`] measures), so each row has a piece of each. Ids
/// the page does not have make it false.
pub fn shares_rows(page: &PageIn, unsure: &[PrimId], other: &[PrimId]) -> bool {
    crate::work::add(1 + unsure.len() as u64 + other.len() as u64);
    let inked = |prims: &[PrimId]| -> Option<Vec<&TextRun>> {
        let mut runs = Vec::new();
        for p in prims {
            if let PrimId::Text(i) = *p {
                let r = page.texts.get(i as usize)?;
                if r.off_axis {
                    return None;
                }
                if !r.text.trim().is_empty() {
                    runs.push(r);
                }
            }
        }
        (!runs.is_empty()).then_some(runs)
    };
    let (Some(a), Some(b)) = (inked(unsure), inked(other)) else {
        return false;
    };
    // The first run of each row (by its left edge) and the row's line number, top down.
    let rows = |runs: &[&TextRun]| -> Vec<(u32, f32, f32)> {
        let mut first: BTreeMap<u32, (f32, f32, f32)> = BTreeMap::new();
        for r in runs {
            let left = r.x.min(r.x + r.w);
            let e = first.entry(r.line).or_insert((left, r.y, r.size));
            if left < e.0 {
                *e = (left, r.y, r.size);
            }
        }
        first.into_iter().map(|(l, (_, y, s))| (l, y, s)).collect()
    };
    let (ra, rb) = (rows(&a), rows(&b));
    if ra.len() != rb.len() || ra.len() < MIN_SHARED_ROWS {
        return false;
    }
    let same_rows = ra.iter().zip(&rb).all(|(&(la, ya, sa), &(lb, yb, sb))| {
        la == lb && (ya - yb).abs() <= ROW_BASELINE * sa.max(sb)
    });
    if !same_rows {
        return false;
    }
    let extent = |runs: &[&TextRun]| {
        runs.iter().fold((f32::MAX, f32::MIN), |(l, r), t| {
            (l.min(t.x.min(t.x + t.w)), r.max(t.x.max(t.x + t.w)))
        })
    };
    let ((a0, a1), (b0, b1)) = (extent(&a), extent(&b));
    let gap = (a0 - b1).max(b0 - a1);
    let adv = advance(a.iter().chain(&b).copied());
    gap.is_finite() && gap >= SPATIAL_PIECE_GAP * adv
}

/// Two regions are lines of one column when their ink overlaps across at least this share of
/// the width of the narrower one (see [`interleaves`]).
const INTERLEAVE_OVERLAP: f32 = 0.5;

/// Whether the text runs `unsure` (a region whose kind is not certain) and `other` (another
/// such region) are lines of one column cut apart: each has its own rows (no line number in
/// both), a row of one stands between two rows of the other, and the ink of the two overlaps
/// across most of the narrower one ([`INTERLEAVE_OVERLAP`]). Read one after the other, a line of one would come after lines it stands below; read
/// row by row together they keep the order they stand in (a note line between the lines of a
/// list of reasons that a change of face cut into two regions).
///
/// Positive evidence only: both have runs with ink, none off the line's axis. Ids the page does
/// not have make it false.
pub fn interleaves(page: &PageIn, unsure: &[PrimId], other: &[PrimId]) -> bool {
    crate::work::add(1 + unsure.len() as u64 + other.len() as u64);
    let inked = |prims: &[PrimId]| -> Option<(Vec<u32>, f32, f32)> {
        let mut lines = Vec::new();
        let (mut x0, mut x1) = (f32::MAX, f32::MIN);
        for p in prims {
            if let PrimId::Text(i) = *p {
                let r = page.texts.get(i as usize)?;
                if r.off_axis {
                    return None;
                }
                if !r.text.trim().is_empty() {
                    lines.push(r.line);
                    x0 = x0.min(r.x.min(r.x + r.w));
                    x1 = x1.max(r.x.max(r.x + r.w));
                }
            }
        }
        lines.sort_unstable();
        lines.dedup();
        (!lines.is_empty()).then_some((lines, x0, x1))
    };
    let (Some((la, a0, a1)), Some((lb, b0, b1))) = (inked(unsure), inked(other)) else {
        return false;
    };
    if la.iter().any(|l| lb.binary_search(l).is_ok()) || a1 <= b0 || b1 <= a0 {
        return false;
    }
    // One column: the ink of the two overlaps across most of the narrower one, not at an edge.
    let overlap = a1.min(b1) - a0.max(b0);
    if overlap < INTERLEAVE_OVERLAP * (a1 - a0).min(b1 - b0) {
        return false;
    }
    let between = |rows: &[u32], of: &[u32]| {
        let (Some(first), Some(last)) = (of.first(), of.last()) else {
            return false;
        };
        rows.iter().any(|l| first < l && l < last)
    };
    between(&la, &lb) || between(&lb, &la)
}

/// The median advance per character of `runs` (over runs with width and characters); half
/// their median text size without such a run, and 1 without a size either.
fn advance<'a>(runs: impl Iterator<Item = &'a TextRun>) -> f32 {
    let mut per: Vec<f32> = Vec::new();
    let mut sizes: Vec<f32> = Vec::new();
    for r in runs {
        let chars = r.text.chars().count();
        let w = r.w.abs();
        if chars > 0 && w.is_finite() && w > 0.0 {
            per.push(w / chars as f32);
        }
        if r.size.is_finite() && r.size > 0.0 {
            sizes.push(r.size);
        }
    }
    let median = |v: &mut Vec<f32>| {
        v.sort_by(f32::total_cmp);
        v.get(v.len() / 2).copied()
    };
    median(&mut per)
        .or_else(|| median(&mut sizes).map(|s| s / 2.0))
        .unwrap_or(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{FontKind, Shift};

    fn page(texts: Vec<TextRun>) -> PageIn {
        PageIn {
            page: 1,
            size: [595.0, 842.0],
            turn: 0,
            texts,
            rules: Vec::new(),
            boxes: Vec::new(),
            pictures: Vec::new(),
            codes: Vec::new(),
            given: Vec::new(),
        }
    }

    /// A run of `text` at `x` on line `line`, 5 points per character.
    fn run(text: &str, x: f32, line: u32) -> TextRun {
        TextRun {
            text: text.into(),
            x,
            y: 700.0 - 12.0 * line as f32,
            w: 5.0 * text.chars().count() as f32,
            h: 10.0,
            size: 10.0,
            font: 1,
            font_kind: FontKind::Proportional,
            bold: false,
            italic: false,
            color: None,
            shift: Shift::Base,
            line,
            off_axis: false,
            gaps: Vec::new(),
        }
    }

    fn off_axis(r: TextRun) -> TextRun {
        TextRun {
            off_axis: true,
            ..r
        }
    }

    fn text(i: u32) -> PrimId {
        PrimId::Text(i)
    }

    /// A run of `text` at `x` on the baseline `y`, in the size `size`, half a size per
    /// character.
    fn sized(text: &str, x: f32, y: f32, size: f32, line: u32) -> TextRun {
        TextRun {
            y,
            w: size / 2.0 * text.chars().count() as f32,
            h: size,
            size,
            ..run(text, x, line)
        }
    }

    #[test]
    fn frame_is_what_the_reading_counts_its_columns_in() {
        // Two lines 12 points apart and a third 30 under them: the pitch is the lower step.
        // The left edge and the advance are those of the reading itself.
        let p = page(vec![
            run("Name:", 72.0, 0),
            run("Ort:", 272.0, 0),
            run("  ", 60.0, 1),
            run("Erika", 72.0, 1),
            TextRun {
                y: 646.0,
                ..run("Summe", 72.0, 2)
            },
        ]);
        let prims: Vec<PrimId> = (0..5).map(text).collect();
        let f = frame(&p, &prims).unwrap();
        assert_eq!((f.x0, f.adv, f.size), (60.0, 5.0, 10.0));
        assert_eq!((f.first, f.last, f.pitch), (700.0, 646.0, Some(12.0)));
        // The columns of the reading are counted from that edge in that advance.
        let lines = spatial(&p, &prims);
        assert_eq!(
            lines[0].pieces[1].0,
            ((272.0 - f.x0) / f.adv).round() as u32
        );
        // One line has no pitch; no text run, no frame.
        let one = frame(&p, &[text(0), text(1)]).unwrap();
        assert_eq!((one.first, one.last, one.pitch), (700.0, 700.0, None));
        assert_eq!(frame(&p, &[]), None);
        assert_eq!(frame(&p, &[PrimId::Text(99)]), None);
        // The grid of the page: its leftmost text and the advance of all its text. Three
        // rows show no pitch.
        assert_eq!(
            page_grid(&p),
            Some(PageGrid {
                left: 60.0,
                unit: 5.0,
                pitch: None
            })
        );
        assert_eq!(page_grid(&page(Vec::new())), None);
    }

    /// A page of one run of 7 points on each of the baselines `ys`.
    fn rows_at(ys: &[f32]) -> PageIn {
        page(
            ys.iter()
                .enumerate()
                .map(|(i, &y)| sized("Row", 72.0, y, 7.0, i as u32))
                .collect(),
        )
    }

    #[test]
    fn a_page_shows_a_line_pitch_when_most_of_its_rows_step_by_it() {
        // A listing: text of 7 points every 12 points, with an empty line here and there.
        let listing = rows_at(&[700.0, 688.0, 676.0, 652.0, 640.0, 628.0, 604.0]);
        assert_eq!(line_pitch(&listing), Some(12.0));
        assert_eq!(page_grid(&listing).unwrap().pitch, Some(12.0));
        // PITCH_STEPS, both ways: three equal steps are a pitch, two are not.
        assert_eq!(
            line_pitch(&rows_at(&[700.0, 688.0, 676.0, 664.0])),
            Some(12.0)
        );
        assert_eq!(line_pitch(&rows_at(&[700.0, 688.0, 676.0])), None);
        // More than half of the steps, both ways: 4 of 6 steps of 12 points are the pitch,
        // 3 of 6 are not.
        assert_eq!(
            line_pitch(&rows_at(&[700.0, 688.0, 676.0, 664.0, 652.0, 600.0, 500.0])),
            Some(12.0)
        );
        assert_eq!(
            line_pitch(&rows_at(&[700.0, 688.0, 676.0, 664.0, 630.0, 580.0, 500.0])),
            None
        );
        // Lone lines far apart at steps of their own show none.
        assert_eq!(
            line_pitch(&rows_at(&[700.0, 640.0, 564.0, 459.0, 300.0])),
            None
        );
        // ROW_BASELINE, both ways: runs on one row (1.5 points apart at 7 points) are one
        // baseline and steps that near are one step; 2 points apart they are rows of their
        // own, and the steps of 2 and 10 points are no pitch.
        assert_eq!(
            line_pitch(&rows_at(&[700.0, 698.5, 688.0, 676.0, 662.5])),
            Some(12.0)
        );
        assert_eq!(
            line_pitch(&rows_at(&[700.0, 698.0, 688.0, 686.0, 676.0])),
            None
        );
        // Text off the baseline or without ink is no row.
        let mut raised = rows_at(&[700.0, 688.0, 676.0, 664.0]);
        raised.texts.push(TextRun {
            shift: Shift::Raised,
            ..sized("2", 90.0, 694.0, 5.0, 9)
        });
        raised.texts.push(sized("  ", 90.0, 682.0, 7.0, 10));
        assert_eq!(line_pitch(&raised), Some(12.0));
        assert_eq!(line_pitch(&page(Vec::new())), None);
    }

    /// The frame of one line of two pieces at `x` on the baseline `y` in the size `size`.
    fn row(x: f32, y: f32, size: f32) -> SpatialFrame {
        let p = page(vec![
            sized("Label", x, y, size, 0),
            sized("Value", x + 20.0 * size, y, size, 0),
        ]);
        frame(&p, &[text(0), text(1)]).unwrap()
    }

    #[test]
    fn a_row_is_under_another_in_one_column_and_within_two_pitches() {
        // Columns of the page 5 points wide; rows in 10 points, so a line pitch of one line
        // is 12 points (RUN_LEADING).
        let unit = 5.0;
        let grid = |unit: f32| PageGrid {
            left: 40.0,
            unit,
            pitch: None,
        };
        let top = row(72.0, 700.0, 10.0);
        let under = |x: f32, y: f32| row(x, y, 10.0).under(&top, top.x0, &grid(unit));
        // Right under it, and with one empty row between (two pitches).
        assert!(under(72.0, 688.0));
        assert!(under(72.0, 676.0));
        // SAME_COLUMN, both ways: less than a column of the page beside the edge of the run
        // it is the same column, a column or more it is another one.
        assert!(under(72.0 + 0.9 * unit, 688.0));
        assert!(under(72.0 - 0.9 * unit, 688.0));
        assert!(!under(72.0 + 1.0 * unit, 688.0));
        assert!(!under(72.0 - 1.1 * unit, 688.0));
        assert!(!under(272.0, 688.0));
        // The column is the page's: in a page of wider characters the same edges are one.
        assert!(row(78.0, 688.0, 10.0).under(&top, top.x0, &grid(7.0)));
        // The edge is that of the run's first block, not of the block above.
        assert!(!row(72.0, 688.0, 10.0).under(&top, 80.0, &grid(unit)));
        // RUN_PITCHES, both ways: two pitches (24 points) are a row of the run, more is not.
        assert!(under(72.0, 700.0 - RUN_PITCHES * RUN_LEADING * 10.0));
        assert!(!under(72.0, 700.0 - RUN_PITCHES * RUN_LEADING * 10.0 - 0.5));
        assert!(!under(72.0, 700.0 - 3.0 * RUN_LEADING * 10.0));
        assert!(!under(72.0, 470.0));
        // ROW_BASELINE, both ways: on the same baseline, or over it, it is no row under it.
        assert!(!under(72.0, 700.0));
        assert!(!under(72.0, 700.0 - ROW_BASELINE * 10.0));
        assert!(under(72.0, 700.0 - ROW_BASELINE * 10.0 - 0.1));
        assert!(!under(72.0, 712.0));
        // The same page at twice the size reads alike: sizes, places and the unit double.
        let big = row(144.0, 1400.0, 20.0);
        assert!(row(144.0 + 1.8 * unit, 1352.0, 20.0).under(&big, big.x0, &grid(2.0 * unit)));
        assert!(!row(144.0 + 2.0 * unit, 1352.0, 20.0).under(&big, big.x0, &grid(2.0 * unit)));
        assert!(!row(144.0, 1351.0, 20.0).under(&big, big.x0, &grid(2.0 * unit)));
        // The larger size of the two counts: small labels over larger values.
        assert!(row(72.0, 700.0 - 24.0, 10.0).under(&row(72.0, 700.0, 7.5), 72.0, &grid(unit)));
        // A block of lines at a wide pitch of its own: two of its pitches.
        let p = page(vec![
            sized("Label", 72.0, 700.0, 10.0, 0),
            sized("Value", 272.0, 700.0, 10.0, 0),
            sized("Label", 72.0, 680.0, 10.0, 1),
        ]);
        let wide = frame(&p, &[text(0), text(1), text(2)]).unwrap();
        assert_eq!(wide.pitch, Some(20.0));
        assert!(row(72.0, 640.0, 10.0).under(&wide, wide.x0, &grid(unit)));
        assert!(!row(72.0, 639.0, 10.0).under(&wide, wide.x0, &grid(unit)));
        // No size or no column width: no evidence.
        assert!(!row(72.0, 688.0, 10.0).under(&top, top.x0, &grid(0.0)));
        assert!(!row(72.0, 688.0, 10.0).under(&top, top.x0, &grid(f32::NAN)));
        let none = SpatialFrame { size: 0.0, ..top };
        let sizeless = SpatialFrame {
            size: 0.0,
            ..row(72.0, 688.0, 10.0)
        };
        assert!(!sizeless.under(&none, 72.0, &grid(unit)));
        // The pitch the page shows, both ways: text of 7 points on lines 12 points apart.
        // A row with one empty line over it (24 points) is a row of the run on that page,
        // and none on a page that shows no pitch (two lines of 7 points are 16.8 points);
        // three lines further down it is none on either.
        let listing = PageGrid {
            pitch: Some(12.0),
            ..grid(4.2)
        };
        let small = row(72.0, 700.0, 7.0);
        assert!(row(72.0, 676.0, 7.0).under(&small, 72.0, &listing));
        assert!(!row(72.0, 676.0, 7.0).under(&small, 72.0, &grid(4.2)));
        assert!(!row(72.0, 664.0, 7.0).under(&small, 72.0, &listing));
        // A pitch of the page tighter than a line of the text does not narrow the bound.
        let tight = PageGrid {
            pitch: Some(4.0),
            ..grid(4.2)
        };
        assert!(row(72.0, 684.0, 7.0).under(&small, 72.0, &tight));
    }

    #[test]
    fn columns_of_a_run_are_counted_on_one_grid() {
        // Labels in 7.5 points from 40 and values in 9 points from 42.7: the value that
        // stands under the third label is in its column on the grid of the run (from the
        // least edge, in the least advance), not in the column it has on its own grid.
        let labels = SpatialFrame {
            x0: 40.0,
            adv: 3.75,
            ..row(40.0, 640.0, 7.5)
        };
        let values = SpatialFrame {
            x0: 42.7,
            adv: 4.5,
            ..row(42.7, 629.2, 9.0)
        };
        let (x0, adv) = (40.0, 3.75);
        // The third label at 205 is column 44 of the labels; the value at 207.7 is column
        // 37 of the values' own grid.
        let label = ((205.0_f32 - 40.0) / 3.75).round() as u32;
        let value = ((207.7_f32 - 42.7) / 4.5).round() as u32;
        assert_eq!((label, value), (44, 37));
        assert_eq!(labels.column_on(label, x0, adv), 44);
        assert_eq!(values.column_on(value, x0, adv), 45);
        assert_eq!(values.column_on(0, x0, adv), 1);
        // Left of the grid's edge is column 0.
        assert_eq!(labels.column_on(0, 60.0, adv), 0);
    }

    #[test]
    fn spatial_keeps_every_run_once_in_order() {
        // Two columns of label-like lines (at 72 and 272), a line across both in between, and
        // a run that follows its line's first run closely (one piece). The page lists the runs
        // out of order.
        let texts = vec![
            run("Ort:", 272.0, 0),                                             // 0
            run("Name:", 72.0, 0),                                             // 1
            run("Muster", 102.0, 0), // 2: one advance after "Name:" ends at 97
            run("Musterstadt", 297.0, 0), // 3
            run("Erika Mustermann wohnt in Musterstadt seit Jahren", 72.0, 1), // 4
            run("PLZ:", 72.0, 2),    // 5
            run("12345", 272.0, 2),  // 6
            run(" ", 400.0, 2),      // 7: white space keeps its place
            off_axis(run("Muster", 500.0, 1)), // 8: off its axis, kept in its line
        ];
        let p = page(texts);
        let mut prims: Vec<PrimId> = (0..9).rev().map(text).collect();
        prims.push(PrimId::Rule(0));
        let got = spatial(&p, &prims);
        // adv is 5; columns from x 72. "Ort:" and "12345" stand in one column (40).
        assert_eq!(
            got,
            [
                SpatialLineOut {
                    pieces: vec![(0, vec![text(1), text(2)]), (40, vec![text(0), text(3)])]
                },
                SpatialLineOut {
                    pieces: vec![(0, vec![text(4)]), (86, vec![text(8)])]
                },
                SpatialLineOut {
                    pieces: vec![(0, vec![text(5)]), (40, vec![text(6)]), (66, vec![text(7)])]
                },
            ]
        );
        let mut all: Vec<PrimId> = got
            .iter()
            .flat_map(|l| l.pieces.iter().flat_map(|(_, p)| p.iter().copied()))
            .collect();
        assert_eq!(all.len(), 9, "every run once");
        all.sort_unstable();
        assert_eq!(all, (0..9).map(text).collect::<Vec<_>>());
        // Columns of one column line up across lines; pieces go left to right.
        for l in &got {
            assert!(l.pieces.windows(2).all(|w| w[0].0 < w[1].0));
        }
    }

    /// Runs without width or size still give finite columns, and each run once.
    #[test]
    fn runs_without_advance_keep_their_place() {
        let mut a = run("Feld", 72.0, 0);
        let mut b = run("Wert", 222.0, 0);
        for r in [&mut a, &mut b] {
            r.w = 0.0;
            r.size = 0.0;
        }
        let got = spatial(&page(vec![a, b]), &[text(0), text(1)]);
        assert_eq!(
            got,
            [SpatialLineOut {
                pieces: vec![(0, vec![text(0)]), (150, vec![text(1)])]
            }]
        );
        assert!(spatial(&page(Vec::new()), &[PrimId::Picture(0)]).is_empty());
    }

    /// A column of lone amounts beside the names on their rows is read with them; a column on
    /// other rows, one with a row more, one that overlaps, one off its axis or one row alone
    /// is not.
    #[test]
    fn lone_amounts_share_the_rows_of_their_names() {
        let texts = vec![
            run("1 Muster Klaus", 280.0, 0), // 0
            run("8,60-", 530.0, 0),          // 1
            run("2 Muster Clara", 280.0, 1), // 2
            run("40,70-", 525.0, 1),         // 3
            run("3 Muster Tom", 280.0, 2),   // 4
            run("55,91-", 525.0, 2),         // 5
            run("Fremd", 525.0, 3),          // 6: a row the names do not have
            run("weit", 505.0, 1),           // 7: a name's word up to the amounts' edge
        ];
        let p = page(texts);
        let names = [text(0), text(2), text(4)];
        let amounts = [text(1), text(3), text(5)];
        assert!(shares_rows(&p, &amounts, &names));
        assert!(shares_rows(&p, &names, &amounts), "either side");
        assert!(!shares_rows(
            &p,
            &[text(1), text(3), text(5), text(6)],
            &names
        ));
        assert!(!shares_rows(&p, &[text(1)], &[text(0)]), "one row alone");
        let reaching = [text(0), text(2), text(4), text(7)];
        assert!(!shares_rows(&p, &amounts, &reaching), "no gap between them");
        let mut moved = p.clone();
        moved.texts[5].y -= 6.0;
        assert!(!shares_rows(&moved, &amounts, &names), "baselines apart");
        let mut turned = p.clone();
        turned.texts[0] = off_axis(turned.texts[0].clone());
        assert!(!shares_rows(&turned, &amounts, &names), "off its axis");
        let close = page(vec![
            run("Muster", 72.0, 0),
            run("1,00", 106.0, 0),
            run("Muster", 72.0, 1),
            run("2,00", 106.0, 1),
        ]);
        assert!(
            !shares_rows(&close, &[text(1), text(3)], &[text(0), text(2)]),
            "a gap under two advances"
        );
    }

    /// Lines of one column cut into two regions, one's line between the other's, interleave;
    /// regions one above the other, side by side, sharing a row or off their axis do not.
    #[test]
    fn lines_of_one_column_cut_apart_interleave() {
        let texts = vec![
            run("Reasons: leave, illness, travel", 72.0, 0),  // 0
            run("Course, parental leave", 96.0, 1),           // 1
            run("or absences", 96.0, 2),                      // 2
            run("Note: do not enter as a break in", 72.0, 3), // 3
            run("service time.", 72.0, 4),                    // 4
            run("Seite 1", 400.0, 2),                         // 5: beside, not under
        ];
        let p = page(texts);
        let reasons = [text(0), text(1), text(3)];
        let cut = [text(2), text(4)];
        assert!(interleaves(&p, &cut, &reasons));
        assert!(interleaves(&p, &reasons, &cut), "either side");
        assert!(
            !interleaves(&p, &[text(3), text(4)], &[text(0), text(1)]),
            "one above the other"
        );
        assert!(
            !interleaves(&p, &[text(5)], &[text(0), text(3)]),
            "side by side"
        );
        assert!(
            !interleaves(&p, &[text(2), text(3)], &[text(1), text(3)]),
            "a row of both"
        );
        let mut turned = p.clone();
        turned.texts[2] = off_axis(turned.texts[2].clone());
        assert!(!interleaves(&turned, &cut, &reasons), "off its axis");
    }

    /// Fix round 1b of Task 19 (M7): two regions whose lines stand between each other's lines
    /// are one column cut apart only when their ink overlaps across most of the narrower one;
    /// an edge of one under the edge of the other is two columns side by side.
    #[test]
    fn regions_that_overlap_at_an_edge_only_do_not_interleave() {
        let texts = vec![
            run("first line of the left column", 72.0, 0), // 0: 72..217
            run("a line of the right column", 205.0, 1),   // 1: 205..335
            run("third line of the left column", 72.0, 2), // 2: 72..217
            run("a line set across both columns", 90.0, 3), // 3: 90..240
        ];
        let p = page(texts);
        assert!(
            !interleaves(&p, &[text(1)], &[text(0), text(2)]),
            "an edge only"
        );
        assert!(
            interleaves(&p, &[text(0), text(2)], &[text(3), text(1)]),
            "most of the narrower one"
        );
    }
}
