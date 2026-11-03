// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The spatial reading of a region whose kind is not certain: its text as it stands, line by
//! line, each line in pieces at their columns, so a person or an agent can still make sense of
//! it. It keeps every text run of the region exactly once, in reading order.

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
}
