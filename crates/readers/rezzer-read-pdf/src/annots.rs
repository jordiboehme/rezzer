// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Annotations in the model, each at its passage.
//!
//! The text a highlight, underline or strike-out marks is quoted from the glyphs whose middle
//! lies inside its quads, while the page's glyphs are still at hand in the text walk; no
//! glyph box outlives its page. Each annotation stands at the block that holds the first
//! quoted glyph, else at the block nearest to the top of its rectangle, else at its page.
//! Replies nest under their parent; members of a group join their parent. Comments become
//! plain inlines, line by line: nothing from the file is read as Markdown.
//!
//! Quotes, authors and comments copied into the model are charged to the bytes the file may
//! keep. Matching visits each glyph near a quad once per quad, and the visits of the whole
//! file are bounded; past a bound what was read stays and one loss names the rest.

use std::collections::HashMap;

use pdfparts::Annot;
use rezzer_limits::Budget;
use rezzer_model::{Anchor, Annotation, Inline, LossReason, Severity, link_target_written_len};

use crate::Out;
use crate::layout::{LinkRoom, Run};
use crate::text::{Gl, Texts};

/// What one quote costs besides its text: its string and its slot.
const ITEM_COST: u64 = 128;

/// What one annotation costs in the model and while it is placed, besides its texts, as
/// measured: the annotation with its lists, its place and its node in the reply tree.
const ANNOT_COST: u64 = 512;

/// The share of the memory limit the annotations may keep at most, besides what the pages
/// leave: the renderer and the JSON writer copy what is kept about four times over.
const OWN_SHARE: u64 = 64;

/// Reply levels kept as nested replies; deeper replies stay at the deepest level, so the
/// rendered lists and the cached model stay within their nesting limits.
const REPLY_DEPTH: usize = 16;

/// Runs on each side of the nearest one that are weighed for a note's block.
const NEAR_WINDOW: usize = 64;

/// Glyphs and runs visited between two looks at the clock.
const POLL: usize = 4096;

/// Words for the subtypes, as the model names annotations.
fn kind_word(subtype: &str) -> String {
    match subtype {
        "Highlight" => "highlight",
        "Underline" => "underline",
        "StrikeOut" => "strikeout",
        "Squiggly" => "squiggly",
        "Text" => "note",
        "FreeText" => "free-text",
        "Ink" => "ink",
        "Caret" => "caret",
        "Stamp" => "stamp",
        "Square" => "square",
        "Circle" => "circle",
        "Line" => "line",
        "Polygon" => "polygon",
        "FileAttachment" => "file-attachment",
        "Sound" => "sound",
        "Redact" => "redact",
        other => return other.to_lowercase(),
    }
    .to_string()
}

/// Review states that close the annotation they answer.
fn closes(state: &str) -> bool {
    matches!(state, "Accepted" | "Completed" | "Cancelled")
}

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

/// Where each annotation of the file stands and what it quotes, once its page is laid out.
#[derive(Debug, Clone, Default)]
pub(crate) struct Placed {
    pub anchor: Option<Anchor>,
    pub quoted: Option<String>,
    /// The quote is made of whole runs, not of the glyphs inside the quads.
    pub from_runs: bool,
    /// A bound left the quote out, which its own loss says.
    pub cut: bool,
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

/// The comment of an annotation as inlines: one plain text per line, with line breaks.
fn body(contents: Option<&str>) -> Vec<Inline> {
    let mut out = Vec::new();
    let Some(text) = contents else {
        return out;
    };
    for (k, line) in text
        .split("\r\n")
        .flat_map(|l| l.split(['\r', '\n']))
        .enumerate()
    {
        if k > 0 {
            out.push(Inline::LineBreak);
        }
        if !line.is_empty() {
            out.push(Inline::plain(line));
        }
    }
    while matches!(out.last(), Some(Inline::LineBreak)) {
        out.pop();
    }
    out
}

/// When an annotation was last changed, else made, in seconds since 1970 UTC; the earliest
/// time there is when it has no date that parses.
fn when(a: &Annot) -> i64 {
    fn iso(d: &Option<pdfparts::PdfDate>) -> Option<&str> {
        d.as_ref().and_then(|d| d.iso.as_deref())
    }
    iso(&a.modified)
        .or_else(|| iso(&a.created))
        .and_then(seconds)
        .unwrap_or(i64::MIN)
}

/// Seconds since 1970 UTC of an ISO 8601 date as pdfparts writes it: `YYYY`, then `-MM`,
/// `-DD`, `THH:MM:SS` as far as given, then `Z` or an offset `+hh:mm`. Without an offset
/// the time counts as UTC.
fn seconds(iso: &str) -> Option<i64> {
    let b = iso.as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        let part = iso.get(from..from + len)?;
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    let year = num(0, 4)?;
    let field = |at: usize, sep: u8, default: i64| match b.get(at) {
        Some(&c) if c == sep => num(at + 1, 2),
        _ => Some(default),
    };
    let month = field(4, b'-', 1)?;
    let day = field(7, b'-', 1)?;
    let hour = field(10, b'T', 0)?;
    let minute = field(13, b':', 0)?;
    let second = field(16, b':', 0)?;
    let offset = match b.get(19) {
        Some(&c @ (b'+' | b'-')) => {
            let sign = if c == b'-' { -1 } else { 1 };
            sign * (num(20, 2)? * 3600 + num(23, 2).unwrap_or(0) * 60)
        }
        _ => 0,
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// The date: when it was last changed, else when it was made, in ISO 8601.
fn date(a: &Annot) -> Option<String> {
    let iso = |d: &Option<pdfparts::PdfDate>| d.as_ref().and_then(|d| d.iso.clone());
    iso(&a.modified).or_else(|| iso(&a.created))
}

/// Puts every annotation of the file in the model, replies under their parent. `placed`
/// holds where each stands and what it quotes, by its index in `annots`.
pub(crate) fn read(
    annots: &[Annot],
    placed: Vec<Placed>,
    budget: &Budget,
    room: &LinkRoom,
    out: &mut Out,
) {
    if annots.is_empty() {
        return;
    }
    let n = annots.len();
    // Each annotation and what it costs, built in file order; the first that does not fit
    // ends the list.
    let mut built: Vec<Option<Annotation>> = Vec::with_capacity(n);
    let mut cut: Option<(usize, String)> = None;
    let mut own = budget.limits().max_memory_bytes / OWN_SHARE;
    for (i, (a, p)) in annots.iter().zip(placed).enumerate() {
        if i % POLL == 0
            && let Err(hit) = budget.check_time()
        {
            cut = Some((i + 1, hit.to_string()));
            break;
        }
        let mut b = body(a.contents.as_deref());
        if p.from_runs && p.quoted.is_some() {
            if !b.is_empty() {
                b.push(Inline::plain(" "));
            }
            b.push(Inline::plain("(quote from whole text runs)"));
        }
        let kind = kind_word(&a.subtype);
        let date = date(a);
        let anchor = p.anchor.unwrap_or_else(|| Anchor(format!("p{}", a.page)));
        let resolved = a.state.as_deref().filter(|s| closes(s)).map(|_| true);
        // The quote was charged to the room when it was read; it counts toward the share.
        let bytes = [
            kind.as_str(),
            a.author.as_deref().unwrap_or(""),
            date.as_deref().unwrap_or(""),
            anchor.0.as_str(),
            a.contents.as_deref().unwrap_or(""),
        ]
        .iter()
        .map(|t| link_target_written_len(t))
        .fold(ANNOT_COST, u64::saturating_add);
        let share = bytes.saturating_add(p.quoted.as_deref().map_or(0, link_target_written_len));
        if share > own {
            cut = Some((
                i + 1,
                format!(
                    "the copies take more than the {own} bytes left of what the annotations may \
                     keep (a 64th of the memory limit)"
                ),
            ));
            break;
        }
        if bytes > room.left() {
            cut = Some((
                i + 1,
                format!(
                    "the copies take more than the {} bytes left of what the file may keep",
                    room.left()
                ),
            ));
            break;
        }
        if let Err(hit) = budget.add_elements(1 + b.len() as u64) {
            cut = Some((i + 1, hit.to_string()));
            break;
        }
        let taken = room.take_kept(bytes);
        debug_assert!(taken, "checked above");
        own -= share;
        built.push(Some(Annotation {
            kind,
            author: a.author.as_deref().map(str::to_string),
            date,
            anchor: Some(anchor),
            quoted: p.quoted,
            body: b,
            replies: Vec::new(),
            resolved,
        }));
    }
    let kept = built.len();
    let tree = nest(&annots[..kept]);
    out.doc.facets.annotations = tree.build(&mut built, &annots[..kept]);
    if let Some((first, why)) = cut {
        let what = if first == n {
            format!("annotation {first} is")
        } else {
            format!("annotations {first} to {n} are")
        };
        out.loss(
            "document",
            format!("{what} not kept: {why}"),
            LossReason::Limit,
            Severity::Warning,
        );
    }
    if tree.loops > 0 {
        let k = tree.loops;
        out.loss(
            "document",
            format!(
                "{k} {} in a loop of replies; {} listed without {} parent",
                if k == 1 {
                    "annotation replies"
                } else {
                    "annotations reply"
                },
                if k == 1 { "it is" } else { "they are" },
                if k == 1 { "its" } else { "their" },
            ),
            LossReason::Damaged,
            Severity::Warning,
        );
    }
    if tree.flattened > 0 {
        let k = tree.flattened;
        out.loss(
            "document",
            format!(
                "{k} {} deeper than {REPLY_DEPTH} levels {} at level {REPLY_DEPTH}",
                if k == 1 { "reply" } else { "replies" },
                if k == 1 { "is shown" } else { "are shown" },
            ),
            LossReason::Unsupported,
            Severity::Info,
        );
    }
}

/// The reply tree of a list of annotations.
struct Tree {
    /// Children of each annotation, in file order, and the annotations without a parent.
    children: Vec<Vec<usize>>,
    roots: Vec<usize>,
    /// Members of a group, which join their parent instead of replying to it.
    group: Vec<bool>,
    loops: usize,
    flattened: usize,
}

/// Who replies to whom. A reply whose parent is not listed stands alone; a loop of replies is
/// broken where the walk meets it again; a reply deeper than [`REPLY_DEPTH`] hangs from the
/// ancestor at that depth. Every walk is a loop, so a long chain cannot overflow the stack.
fn nest(annots: &[Annot]) -> Tree {
    let n = annots.len();
    let by_id: HashMap<&str, usize> = annots
        .iter()
        .enumerate()
        .map(|(i, a)| (a.id.as_str(), i))
        .collect();
    let mut parent: Vec<Option<usize>> = annots
        .iter()
        .enumerate()
        .map(|(i, a)| {
            a.in_reply_to
                .as_deref()
                .and_then(|p| by_id.get(p).copied())
                .filter(|&p| p != i)
        })
        .collect();
    let group: Vec<bool> = annots
        .iter()
        .map(|a| a.reply_type.as_deref() == Some("Group"))
        .collect();
    // Loops are broken first. 0: not seen, 1: on the walk now, 2: done.
    let mut state = vec![0u8; n];
    let mut loops = 0;
    let mut path: Vec<usize> = Vec::new();
    for start in 0..n {
        let mut at = start;
        while state[at] == 0 {
            state[at] = 1;
            path.push(at);
            match parent[at] {
                Some(p) if state[p] == 1 => {
                    // The reply that closes the loop loses its parent.
                    parent[at] = None;
                    loops += 1;
                    break;
                }
                Some(p) => at = p,
                None => break,
            }
        }
        for i in path.drain(..) {
            state[i] = 2;
        }
    }
    // Then the depths, walking up to the first annotation whose depth is known.
    let mut depth = vec![0usize; n];
    // The ancestor at the deepest level that keeps replies, for annotations at or below it.
    let mut top: Vec<usize> = (0..n).collect();
    let mut known = vec![false; n];
    for start in 0..n {
        let mut at = start;
        while !known[at] {
            path.push(at);
            match parent[at] {
                Some(p) => at = p,
                None => break,
            }
        }
        while let Some(i) = path.pop() {
            depth[i] = match parent[i] {
                Some(p) if known[p] => depth[p] + 1,
                _ => 0,
            };
            if depth[i] >= REPLY_DEPTH
                && let Some(p) = parent[i]
            {
                top[i] = top[p];
            }
            known[i] = true;
        }
    }
    let mut flattened = 0;
    // Deep replies hang from their ancestor at the deepest level kept.
    let mut hang: Vec<Option<usize>> = parent.clone();
    for i in 0..n {
        if depth[i] > REPLY_DEPTH
            && let Some(p) = parent[i]
        {
            hang[i] = Some(top[p]);
            flattened += 1;
        }
    }
    let mut children = vec![Vec::new(); n];
    let mut roots = Vec::new();
    for (i, p) in hang.iter().enumerate() {
        match p {
            Some(p) => children[*p].push(i),
            None => roots.push(i),
        }
    }
    Tree {
        children,
        roots,
        group,
        loops,
        flattened,
    }
}

impl Tree {
    /// The annotations as a tree: each root with its replies. Members of a group add their
    /// comment and replies to their parent; a reply that closes its parent marks it
    /// resolved.
    fn build(&self, built: &mut [Option<Annotation>], annots: &[Annot]) -> Vec<Annotation> {
        let mut out = Vec::with_capacity(self.roots.len());
        for &r in &self.roots {
            if let Some(a) = self.node(r, built, annots, 0) {
                out.push(a);
            }
        }
        out
    }

    /// One annotation with its replies. The depth of the tree is at most [`REPLY_DEPTH`]
    /// plus one, so the recursion is bounded.
    fn node(
        &self,
        i: usize,
        built: &mut [Option<Annotation>],
        annots: &[Annot],
        depth: usize,
    ) -> Option<Annotation> {
        let mut a = built[i].take()?;
        let mut latest: Option<((i64, usize), bool)> = None;
        // Group members join here, with their own members and replies.
        let mut todo: Vec<usize> = self.children[i].iter().rev().copied().collect();
        while let Some(c) = todo.pop() {
            if self.group[c] && depth <= REPLY_DEPTH {
                if let Some(member) = built[c].take() {
                    if !member.body.is_empty() {
                        if !a.body.is_empty() {
                            a.body.push(Inline::LineBreak);
                        }
                        a.body.extend(member.body);
                    }
                    if a.quoted.is_none() {
                        a.quoted = member.quoted;
                    }
                    a.resolved = a.resolved.or(member.resolved);
                }
                todo.extend(self.children[c].iter().rev());
                continue;
            }
            // The latest review state counts: by date, else by place in the file.
            if let Some(state) = annots[c].state.as_deref() {
                let key = (when(&annots[c]), c);
                if latest.as_ref().is_none_or(|(k, _)| key > *k) {
                    latest = Some((key, closes(state)));
                }
            }
            if let Some(reply) = self.node(c, built, annots, depth + 1) {
                a.replies.push(reply);
            }
        }
        if let Some((_, closed)) = latest {
            a.resolved = closed.then_some(true);
        }
        Some(a)
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
    fn dates_compare_across_offsets() {
        assert_eq!(seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(seconds("1970-01-01T01:00:00+01:00"), Some(0));
        assert_eq!(seconds("1969-12-31T23:00:00-01:00"), Some(0));
        assert_eq!(seconds("2024-03-01"), Some(1_709_251_200));
        assert_eq!(seconds("x"), None);
    }

    #[test]
    fn comments_keep_their_lines() {
        assert_eq!(
            body(Some("a\r\nb\rc\n")),
            [
                Inline::plain("a"),
                Inline::LineBreak,
                Inline::plain("b"),
                Inline::LineBreak,
                Inline::plain("c")
            ]
        );
        assert!(body(None).is_empty());
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
