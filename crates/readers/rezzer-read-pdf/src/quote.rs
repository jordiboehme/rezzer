// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The text under annotations and the points their blocks are found by.
//!
//! The text a highlight, underline or strike-out marks is quoted from the glyphs whose middle
//! lies inside its quads, while the page's glyphs are still at hand in the text walk; no
//! glyph box outlives its page. Quotes are charged to the bytes the file may keep. Matching
//! visits each glyph near a quad once per quad, and the visits of the whole file are bounded;
//! past a bound what was read stays and one loss names the rest.

use std::collections::HashMap;

use pdfparts::Annot;
use rezzer_model::{LossReason, Severity, link_target_written_len};

use crate::Out;
use crate::layout::Run;
use crate::text::{Gl, Texts};

/// What one quote costs besides its text: its string and its slot.
const ITEM_COST: u64 = 128;
/// Runs on each side of the nearest one that are weighed for a note's block.
const NEAR_WINDOW: usize = 64;

/// Glyphs and runs visited between two looks at the clock.
const POLL: usize = 4096;
/// The annotations of each page that mark text: their index in the file's list and their
/// quads.
pub(crate) type Marked<'a> = HashMap<u32, Vec<(usize, &'a [[f32; 8]])>>;

pub(crate) fn marked(annots: &[Annot]) -> Marked<'_> {
    let mut out: Marked<'_> = HashMap::new();
    for (i, a) in annots.iter().enumerate() {
        if !a.quads.is_empty() {
            out.entry(a.page).or_default().push((i, &a.quads));
        }
    }
    out
}

/// The quoted text of one annotation and where its first glyph stands, in the frame of the
/// page's runs.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Quote {
    pub annot: usize,
    pub text: Option<String>,
    pub at: Option<(f32, f32)>,
    /// A bound left the quote out; [`Quoting::cut`] says why.
    pub cut: bool,
}

/// What quoting may still do in the whole file, and what it could not do.
#[derive(Debug)]
pub(crate) struct Quoting {
    /// Glyph and quad visits left.
    left: usize,
    max: usize,
    /// Annotations whose text was not quoted because the visits ran out, the time was up or
    /// the quote did not fit what the file may keep, and the first page with one.
    pub cut: Option<Cut>,
}

/// Why quotes were left out, how many and from which page on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cut {
    pub why: CutWhy,
    pub count: usize,
    pub page: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CutWhy {
    Visits,
    Time,
    Kept,
}

impl Quoting {
    pub(crate) fn new(max_items: usize) -> Quoting {
        Quoting {
            left: max_items,
            max: max_items,
            cut: None,
        }
    }

    fn take(&mut self, n: usize) -> bool {
        if n > self.left {
            self.left = 0;
            return false;
        }
        self.left -= n;
        true
    }

    fn cut(&mut self, why: CutWhy, page: u32) {
        match &mut self.cut {
            Some(c) => c.count += 1,
            None => {
                self.cut = Some(Cut {
                    why,
                    count: 1,
                    page,
                })
            }
        }
    }

    /// The bound on visits.
    pub(crate) fn max(&self) -> usize {
        self.max
    }
}

/// The middle of a glyph in the frame of the page box: half its advance along the baseline,
/// a quarter of its size above it.
fn middle(g: &Gl) -> (f32, f32) {
    let adv = g.adv.unwrap_or(g.size / 2.0);
    let (ux, uy) = (-g.dy, g.dx);
    (
        g.x + g.dx * adv / 2.0 + ux * g.size / 4.0,
        g.y + g.dy * adv / 2.0 + uy * g.size / 4.0,
    )
}

/// A quad as a convex polygon in the frame of the page box, and its bounding box. Writers
/// order the four corners differently, so the corners are put in order around their middle.
struct Area {
    corners: [(f32, f32); 4],
    bbox: [f32; 4],
}

impl Area {
    fn new(q: &[f32; 8], origin: (f32, f32)) -> Area {
        let mut corners = [(0.0, 0.0); 4];
        for (k, c) in corners.iter_mut().enumerate() {
            *c = (q[2 * k] - origin.0, q[2 * k + 1] - origin.1);
        }
        let mx = corners.iter().map(|c| c.0).sum::<f32>() / 4.0;
        let my = corners.iter().map(|c| c.1).sum::<f32>() / 4.0;
        corners.sort_by(|a, b| {
            (a.1 - my)
                .atan2(a.0 - mx)
                .total_cmp(&(b.1 - my).atan2(b.0 - mx))
        });
        let xs = corners.map(|c| c.0);
        let ys = corners.map(|c| c.1);
        let min = |v: [f32; 4]| v.into_iter().fold(f32::INFINITY, f32::min);
        let max = |v: [f32; 4]| v.into_iter().fold(f32::NEG_INFINITY, f32::max);
        Area {
            corners,
            bbox: [min(xs), min(ys), max(xs), max(ys)],
        }
    }

    fn holds(&self, (x, y): (f32, f32)) -> bool {
        let [x0, y0, x1, y1] = self.bbox;
        if !(x >= x0 && x <= x1 && y >= y0 && y <= y1) {
            return false;
        }
        // Inside a convex polygon all edges turn the same way around the point.
        let mut sign = 0.0f32;
        for k in 0..4 {
            let (ax, ay) = self.corners[k];
            let (bx, by) = self.corners[(k + 1) % 4];
            let cross = (bx - ax) * (y - ay) - (by - ay) * (x - ax);
            if cross.abs() <= f32::EPSILON {
                continue;
            }
            if sign == 0.0 {
                sign = cross.signum();
            } else if cross.signum() != sign {
                return false;
            }
        }
        true
    }
}

/// Quotes the text under each annotation of `marked` from the glyphs of page `page`, the
/// glyphs the body reads: glyphs of switched-off layers are not on the page and are left
/// out, and so is text that puts no glyphs on the page, unless the page shows nothing else
/// (an OCR layer behind a scan). Each quote is charged to `kept_left`
/// (the bytes the file may keep); a stop from `stop` or the visit bound leaves the rest of
/// the page's quotes out.
#[allow(clippy::too_many_arguments)]
pub(crate) fn quote_page(
    page: u32,
    glyphs: &[Gl],
    texts: &Texts<'_>,
    marked: &[(usize, &[[f32; 8]])],
    origin: (f32, f32),
    quoting: &mut Quoting,
    kept_left: &mut u64,
    stop: &dyn Fn() -> bool,
) -> Vec<Quote> {
    let mut out = Vec::with_capacity(marked.len());
    let shown = glyphs.iter().any(|g| g.layer.is_none() && !g.invisible);
    // Each glyph of an ActualText span stands for the span, whose text is on its first glyph:
    // the span is quoted whole when the middle of any of its glyphs lies in a quad.
    let mut head: Vec<u32> = Vec::with_capacity(glyphs.len());
    let mut last = 0u32;
    for (i, g) in glyphs.iter().enumerate() {
        if !g.joined {
            last = i as u32;
        }
        head.push(if g.joined { last } else { i as u32 });
    }
    // The glyphs on the page by the height of their middle.
    let mut by_y: Vec<(f32, f32, u32)> = glyphs
        .iter()
        .enumerate()
        .filter(|(_, g)| g.layer.is_none() && !(shown && g.invisible))
        .map(|(i, g)| {
            let (x, y) = middle(g);
            (y, x, i as u32)
        })
        .filter(|(y, x, _)| y.is_finite() && x.is_finite())
        .collect();
    by_y.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut visits = 0usize;
    let mut halted: Option<CutWhy> = None;
    // For each glyph, the annotation that took it last (counted from 1), so overlapping quads
    // quote a glyph once.
    let mut taken: Vec<u32> = vec![0; glyphs.len()];
    for (k, &(annot, quads)) in marked.iter().enumerate() {
        let mark = k as u32 + 1;
        if let Some(why) = halted {
            quoting.cut(why, page);
            out.push(Quote {
                annot,
                text: None,
                at: None,
                cut: true,
            });
            continue;
        }
        let mut lines: Vec<Vec<u32>> = Vec::new();
        'quads: for q in quads {
            if !quoting.take(1) {
                halted = Some(CutWhy::Visits);
                break;
            }
            let area = Area::new(q, origin);
            let [_, y0, _, y1] = area.bbox;
            let from = by_y.partition_point(|g| g.0 < y0);
            let mut hits: Vec<u32> = Vec::new();
            for &(y, x, i) in &by_y[from..] {
                if y > y1 {
                    break;
                }
                visits += 1;
                if !quoting.take(1) {
                    halted = Some(CutWhy::Visits);
                    break 'quads;
                }
                if visits.is_multiple_of(POLL) && stop() {
                    halted = Some(CutWhy::Time);
                    break 'quads;
                }
                let h = head[i as usize];
                if taken[h as usize] != mark && area.holds((x, y)) {
                    taken[h as usize] = mark;
                    hits.push(h);
                }
            }
            lines.extend(in_lines(glyphs, hits));
        }
        if let Some(why) = halted {
            quoting.cut(why, page);
            out.push(Quote {
                annot,
                text: None,
                at: None,
                cut: true,
            });
            continue;
        }
        let (text, at) = join(glyphs, texts, &lines);
        let mut cut = false;
        let text = (!text.is_empty()).then_some(text).filter(|t| {
            let cost = ITEM_COST + link_target_written_len(t);
            if cost <= *kept_left {
                *kept_left -= cost;
                true
            } else {
                quoting.cut(CutWhy::Kept, page);
                cut = true;
                false
            }
        });
        out.push(Quote {
            annot,
            text,
            at,
            cut,
        });
    }
    out
}

/// The glyphs of one quad as lines, top line first, each in reading order along its
/// baseline.
fn in_lines(glyphs: &[Gl], mut hits: Vec<u32>) -> Vec<Vec<u32>> {
    let Some(&first) = hits.first() else {
        return Vec::new();
    };
    let g0 = &glyphs[first as usize];
    let (dx, dy) = (g0.dx, g0.dy);
    // Distance along the baseline and up from it.
    let along = |g: &Gl| g.x * dx + g.y * dy;
    let up = |g: &Gl| -g.x * dy + g.y * dx;
    hits.sort_by(|a, b| up(&glyphs[*b as usize]).total_cmp(&up(&glyphs[*a as usize])));
    let mut lines: Vec<Vec<u32>> = Vec::new();
    let mut level = f32::NAN;
    for i in hits {
        let g = &glyphs[i as usize];
        let u = up(g);
        match lines.last_mut() {
            Some(line) if (level - u).abs() <= g.size.max(1.0) / 2.0 => line.push(i),
            _ => {
                lines.push(vec![i]);
                level = u;
            }
        }
    }
    for line in &mut lines {
        line.sort_by(|a, b| along(&glyphs[*a as usize]).total_cmp(&along(&glyphs[*b as usize])));
    }
    lines
}

/// The text of the lines of all quads, in order, and the middle of the first glyph. Glyphs
/// on one baseline, in one quad or the next, get a space only where the gap is over a quarter
/// of the glyph height; a new line gets one space, none after a line that ends in a hyphen,
/// as the body joins lines. Runs of white space count as one. A glyph drawn again on top of
/// one of the four before it (a fake bold) is read once, as the body reads it.
fn join(glyphs: &[Gl], texts: &Texts<'_>, lines: &[Vec<u32>]) -> (String, Option<(f32, f32)>) {
    let mut raw = String::new();
    let mut at = None;
    let mut prev: Option<&Gl> = None;
    for line in lines {
        let mut kept: Vec<&Gl> = Vec::with_capacity(line.len());
        for &i in line {
            let g = &glyphs[i as usize];
            // Within half an advance: the same letter twice in a word stands a whole
            // advance on.
            let again = kept.iter().rev().take(4).any(|k| {
                let reach = 0.5 * k.adv.unwrap_or(k.size / 2.0);
                texts.of(k) == texts.of(g)
                    && (k.x - g.x).abs() < reach
                    && (k.y - g.y).abs() < 0.1 * k.size
            });
            if again {
                continue;
            }
            kept.push(g);
            at.get_or_insert_with(|| middle(g));
            match prev {
                None => {}
                Some(p) => {
                    let up = |q: &Gl| -q.x * p.dy + q.y * p.dx;
                    let same_line = (up(p) - up(g)).abs() <= p.size.max(g.size).max(1.0) / 2.0;
                    let end = p.x * p.dx + p.y * p.dy + p.adv.unwrap_or(0.0);
                    let gap = g.x * p.dx + g.y * p.dy - end;
                    if same_line && gap > -0.25 * p.size.max(g.size) {
                        if gap > 0.25 * p.size.max(g.size) {
                            raw.push(' ');
                        }
                    } else if !raw.trim_end().ends_with('-') {
                        raw.push(' ');
                    } else {
                        let end = raw.trim_end().len();
                        raw.truncate(end);
                    }
                }
            }
            raw.push_str(texts.of(g));
            prev = Some(g);
        }
    }
    let text = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    (text, at)
}

/// The quote from whole runs, for a page whose glyphs were not kept: the runs whose middle
/// lies inside a quad, in quad order, and the middle of the first. Coordinates of the quads
/// are in user space; `origin` is the lower left corner of the page box.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn quote_runs(
    runs: &[Run],
    quads: &[[f32; 8]],
    origin: (f32, f32),
) -> Option<(String, (f32, f32))> {
    let mut by_y: Vec<(f32, usize)> = runs
        .iter()
        .enumerate()
        .map(|(i, r)| (r.y + r.h / 4.0, i))
        .collect();
    by_y.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut taken: Vec<usize> = Vec::new();
    let mut seen = vec![false; runs.len()];
    for q in quads {
        let area = Area::new(q, origin);
        let from = by_y.partition_point(|r| r.0 < area.bbox[1]);
        let mut hits: Vec<usize> = by_y[from..]
            .iter()
            .take_while(|r| r.0 <= area.bbox[3])
            .map(|r| r.1)
            .filter(|&i| {
                let r = &runs[i];
                !seen[i] && area.holds((r.x + r.w / 2.0, r.y + r.h / 4.0))
            })
            .collect();
        // Runs come in reading order.
        hits.sort_unstable();
        for &i in &hits {
            seen[i] = true;
        }
        taken.extend(hits);
    }
    let first = &runs[*taken.first()?];
    let text = taken
        .iter()
        .map(|&i| runs[i].text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    Some((text, (first.x + first.w / 2.0, first.y + first.h / 4.0)))
}

/// For each point (x, y in the frame of the runs), the middle of the run nearest to it: the
/// nearest by height, and among runs as near (within [`NEAR_WINDOW`] on each side) the
/// nearest across. `None` when the page has no runs.
pub(crate) fn nearest_runs(runs: &[Run], points: &[(f32, f32)]) -> Vec<Option<(f32, f32)>> {
    if runs.is_empty() {
        return vec![None; points.len()];
    }
    let mut mids: Vec<(f32, f32)> = runs
        .iter()
        .map(|r| (r.y + r.h / 4.0, r.x + r.w / 2.0))
        .filter(|(y, x)| y.is_finite() && x.is_finite())
        .collect();
    if mids.is_empty() {
        return vec![None; points.len()];
    }
    mids.sort_by(|a, b| a.0.total_cmp(&b.0));
    points
        .iter()
        .map(|&(px, py)| {
            let at = mids.partition_point(|m| m.0 < py);
            let lo = at.saturating_sub(NEAR_WINDOW);
            let hi = (at + NEAR_WINDOW).min(mids.len());
            mids[lo..hi]
                .iter()
                .min_by(|a, b| {
                    let da = ((a.0 - py).abs(), (a.1 - px).abs());
                    let db = ((b.0 - py).abs(), (b.1 - px).abs());
                    da.0.total_cmp(&db.0).then(da.1.total_cmp(&db.1))
                })
                .map(|&(y, x)| (x, y))
        })
        .collect()
}
/// Annotations that mark text but are not quoted because their page's text was not read in
/// full, and the first page with one.
#[derive(Debug, Default)]
pub(crate) struct Unquoted {
    pub count: usize,
    pub page: Option<u32>,
}

impl Unquoted {
    pub(crate) fn add(&mut self, page: u32) {
        self.count += 1;
        self.page.get_or_insert(page);
    }
}

/// The losses of quoting: annotations that mark text whose page was not read in full, and
/// quotes a bound left out.
pub(crate) fn losses(unquoted: &Unquoted, cut: Option<Cut>, visits: usize, out: &mut Out) {
    if let Some(page) = unquoted.page {
        let k = unquoted.count;
        out.loss(
            format!("p{page}"),
            format!(
                "{k} {} not quoted, the first on page {page}: the text of {} page was not read \
                 in full",
                if k == 1 {
                    "annotation that marks text is"
                } else {
                    "annotations that mark text are"
                },
                if k == 1 { "its" } else { "their" },
            ),
            LossReason::Damaged,
            Severity::Warning,
        );
    }
    if let Some(c) = cut {
        let why = match c.why {
            CutWhy::Visits => format!("quoting visits more than {visits} glyphs and quads"),
            CutWhy::Time => "the time limit was reached".to_string(),
            CutWhy::Kept => "the quotes take more than the bytes the file may keep".to_string(),
        };
        let k = c.count;
        out.loss(
            format!("p{}", c.page),
            format!(
                "the text under {k} {} is not quoted, from page {} on: {why}",
                if k == 1 { "annotation" } else { "annotations" },
                c.page
            ),
            LossReason::Limit,
            Severity::Warning,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, x: f32, y: f32, line: u32) -> Run {
        Run {
            page: 1,
            text: text.into(),
            x,
            y,
            w: 6.0 * text.len() as f32,
            h: 12.0,
            size: 12.0,
            bold: false,
            italic: false,
            mcid: None,
            uri: None,
            line,
            off_axis: false,
        }
    }

    #[test]
    fn whole_runs_under_the_quads_are_quoted_in_order() {
        let runs = [
            run("Above", 72.0, 720.0, 0),
            run("Line one", 72.0, 700.0, 1),
            run(" and more", 120.0, 700.0, 1),
            run("Line two", 72.0, 680.0, 2),
        ];
        let quad = |y0: f32, y1: f32| [60.0, y1, 300.0, y1, 60.0, y0, 300.0, y0];
        let (text, at) =
            quote_runs(&runs, &[quad(696.0, 712.0), quad(676.0, 692.0)], (0.0, 0.0)).unwrap();
        assert_eq!(text, "Line one and more Line two");
        assert_eq!(at, (72.0 + 24.0, 703.0));
        assert_eq!(quote_runs(&runs, &[quad(0.0, 10.0)], (0.0, 0.0)), None);
    }

    #[test]
    fn a_quad_in_any_corner_order_holds_its_middle() {
        for q in [
            [0.0, 10.0, 10.0, 10.0, 0.0, 0.0, 10.0, 0.0],
            [0.0, 0.0, 10.0, 0.0, 10.0, 10.0, 0.0, 10.0],
            [10.0, 10.0, 0.0, 0.0, 0.0, 10.0, 10.0, 0.0],
        ] {
            let area = Area::new(&q, (0.0, 0.0));
            assert!(area.holds((5.0, 5.0)));
            assert!(!area.holds((11.0, 5.0)));
        }
        // A turned quad leaves out the corners of its bounding box.
        let area = Area::new(&[5.0, 0.0, 10.0, 5.0, 5.0, 10.0, 0.0, 5.0], (0.0, 0.0));
        assert!(area.holds((5.0, 5.0)));
        assert!(!area.holds((1.0, 1.0)));
    }

    #[test]
    fn the_nearest_run_is_the_nearest_line_then_the_nearest_across() {
        let runs = [
            run("left", 72.0, 700.0, 0),
            run("right", 320.0, 700.0, 0),
            run("low", 72.0, 300.0, 1),
        ];
        let near = nearest_runs(&runs, &[(400.0, 705.0), (10.0, 310.0)]);
        assert_eq!(near[0], Some((320.0 + 15.0, 703.0)));
        assert_eq!(near[1], Some((72.0 + 9.0, 303.0)));
        assert_eq!(nearest_runs(&[], &[(0.0, 0.0)]), [None]);
    }
}
