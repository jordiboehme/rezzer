// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The spatial reading of a region whose kind is not certain: its text as it stands, line by
//! line, each line in pieces at their columns, so a person or an agent can still make sense of
//! it. It keeps every text run of the region exactly once, in reading order.

use std::collections::BTreeMap;

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
    let adv = advance(ids.iter().map(|&i| run(i)));
    let x0 = ids.iter().map(|&i| left(run(i))).fold(f32::MAX, f32::min);
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
