// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Drawn lines that meet, and the boxes they draw. A grid of at least two rows and two
//! columns is a table (the ruled detector reads it); a box that stands alone, or a cell of a
//! grid of one row or one column, is a reading region: its text is read whole, before or
//! after the text around it.

use std::collections::HashMap;

use crate::tables::{GRID_WORK, MAX_SEGMENTS, PAGE_SHARE, Rule, RuleKind, SNAP};

/// A straight horizontal (`at` is y, `from`..`to` along x) or vertical segment.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Seg {
    pub at: f32,
    pub from: f32,
    pub to: f32,
}

/// Joined lines a group of segments keeps open for joining at once; past them the oldest is
/// closed. Only a page of many long overlapping lines at slightly different places has more.
const OPEN_LINES: usize = 32;

/// Boxes one page may use as reading regions; past them it uses none and says so.
pub(crate) const MAX_BOXES: usize = 4_096;

/// Steps the box search may take on one page (sides checked, widgets looked at, fragments
/// looked up); past them the page uses no boxes and says so.
pub(crate) const BOX_WORK: usize = 4_000_000;

/// Places in the lookup grid boxes may take together; past them the page uses no boxes.
const INDEX_ENTRIES: usize = 262_144;

/// Cells of the lookup grid along each side.
const INDEX_SIDE: usize = 16;

/// A box side is drawn when lines cover this share of it.
const DRAWN_SIDE: f32 = 0.9;

/// Collinear segments that touch become one. Segments are grouped by where they lie: a
/// segment joins the group when it lies within [`SNAP`] of the group's mean place, so lines
/// in a dense run (bars of a barcode, hatching) never chain far apart. Within a group, a
/// piece joins a line that it overlaps or touches within [`SNAP`] and that lies within
/// [`SNAP`] of it; the joined line lies at the mean of its pieces. Two lines a point apart
/// that never meet along their length stay apart.
pub(crate) fn merged(mut segs: Vec<Seg>) -> Vec<Seg> {
    segs.sort_by(|a, b| a.at.total_cmp(&b.at));
    let mut out: Vec<Seg> = Vec::with_capacity(segs.len());
    let mut start = 0;
    while start < segs.len() {
        let mut end = start + 1;
        let mut sum = f64::from(segs[start].at);
        while end < segs.len() {
            let mean = sum / (end - start) as f64;
            if f64::from(segs[end].at) - mean > f64::from(SNAP) {
                break;
            }
            sum += f64::from(segs[end].at);
            end += 1;
        }
        let group = &mut segs[start..end];
        group.sort_by(|a, b| a.from.total_cmp(&b.from));
        // Open lines: the joined segment, the sum of its pieces' places and their number.
        let mut open: Vec<(Seg, f64, u32)> = Vec::new();
        let close = |(mut s, sum, n): (Seg, f64, u32), out: &mut Vec<Seg>| {
            s.at = (sum / f64::from(n)) as f32;
            out.push(s);
        };
        for s in group.iter() {
            let mut k = 0;
            while k < open.len() {
                if open[k].0.to + SNAP < s.from {
                    close(open.remove(k), &mut out);
                } else {
                    k += 1;
                }
            }
            let near = open.iter_mut().find(|(_, sum, n)| {
                (f64::from(s.at) - sum / f64::from(*n)).abs() <= f64::from(SNAP)
            });
            match near {
                Some((line, sum, n)) => {
                    line.to = line.to.max(s.to);
                    *sum += f64::from(s.at);
                    *n += 1;
                }
                None => {
                    if open.len() == OPEN_LINES {
                        close(open.remove(0), &mut out);
                    }
                    open.push((*s, f64::from(s.at), 1));
                }
            }
        }
        for line in open {
            close(line, &mut out);
        }
        start = end;
    }
    // The grid search looks segments up by where they lie.
    out.sort_by(|a, b| a.at.total_cmp(&b.at).then(a.from.total_cmp(&b.from)));
    out
}

/// Values closer than [`SNAP`] are one, sorted.
pub(crate) fn clustered(mut values: Vec<f32>) -> Vec<f32> {
    values.sort_by(f32::total_cmp);
    let mut out: Vec<f32> = Vec::new();
    for v in values {
        match out.last() {
            Some(l) if v - l <= SNAP => {}
            _ => out.push(v),
        }
    }
    out
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// Why the lines of a page were not searched for grids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unsearched {
    /// More straight segments than [`MAX_SEGMENTS`].
    TooMany,
    /// More pairs of segments to test than [`GRID_WORK`].
    MeetTooOften,
}

impl Unsearched {
    /// Why, in words.
    pub(crate) fn why(self) -> String {
        match self {
            Unsearched::TooMany => {
                format!("the page draws more than {MAX_SEGMENTS} straight lines")
            }
            Unsearched::MeetTooOften => "its lines meet too often".into(),
        }
    }
}

/// The joined segments of a page and the groups of them that meet: indices into `hs` and
/// `vs`, each group with at least two of each.
#[derive(Debug, Default)]
pub(crate) struct Grids {
    pub hs: Vec<Seg>,
    pub vs: Vec<Seg>,
    pub groups: Vec<(Vec<usize>, Vec<usize>)>,
}

/// Horizontal and vertical lines that meet, in groups. A rectangle gives its four sides.
pub(crate) fn grids(rules: &[Rule]) -> Result<Grids, Unsearched> {
    let flat = |r: &Rule| (r.y0 - r.y1).abs() <= SNAP / 2.0;
    // Counted before anything is built: a page may keep many more lines than are searched.
    let (mut h, mut v) = (0usize, 0usize);
    for r in rules {
        match r.kind {
            RuleKind::Line if flat(r) => h += 1,
            RuleKind::Line => v += 1,
            RuleKind::Rect => (h, v) = (h + 2, v + 2),
        }
    }
    if h == 0 || v == 0 {
        return Ok(Grids::default());
    }
    if h + v > MAX_SEGMENTS {
        return Err(Unsearched::TooMany);
    }
    let mut hs = Vec::with_capacity(h);
    let mut vs = Vec::with_capacity(v);
    for r in rules {
        let (x0, x1) = (r.x0.min(r.x1), r.x0.max(r.x1));
        let (y0, y1) = (r.y0.min(r.y1), r.y0.max(r.y1));
        match r.kind {
            RuleKind::Line if flat(r) => hs.push(Seg {
                at: (y0 + y1) / 2.0,
                from: x0,
                to: x1,
            }),
            RuleKind::Line => vs.push(Seg {
                at: (x0 + x1) / 2.0,
                from: y0,
                to: y1,
            }),
            RuleKind::Rect => {
                hs.push(Seg {
                    at: y0,
                    from: x0,
                    to: x1,
                });
                hs.push(Seg {
                    at: y1,
                    from: x0,
                    to: x1,
                });
                vs.push(Seg {
                    at: x0,
                    from: y0,
                    to: y1,
                });
                vs.push(Seg {
                    at: x1,
                    from: y0,
                    to: y1,
                });
            }
        }
    }
    let hs = merged(hs);
    let vs = merged(vs);
    // Segments that meet are one grid. Horizontal ones are sorted by y.
    let n = hs.len();
    let mut parent: Vec<usize> = (0..n + vs.len()).collect();
    let mut work = 0usize;
    for (k, v) in vs.iter().enumerate() {
        let lo = hs.partition_point(|h| h.at < v.from - SNAP);
        for (j, h) in hs.iter().enumerate().skip(lo) {
            if h.at > v.to + SNAP {
                break;
            }
            work += 1;
            if work > GRID_WORK {
                return Err(Unsearched::MeetTooOften);
            }
            if v.at >= h.from - SNAP && v.at <= h.to + SNAP {
                let (a, b) = (find(&mut parent, j), find(&mut parent, n + k));
                parent[a] = b;
            }
        }
    }
    let mut groups: HashMap<usize, (Vec<usize>, Vec<usize>)> = HashMap::new();
    for j in 0..n {
        let root = find(&mut parent, j);
        groups.entry(root).or_default().0.push(j);
    }
    for k in 0..vs.len() {
        let root = find(&mut parent, n + k);
        groups.entry(root).or_default().1.push(k);
    }
    let groups = groups
        .into_values()
        .filter(|(h, v)| h.len() >= 2 && v.len() >= 2)
        .collect();
    Ok(Grids { hs, vs, groups })
}

/// The boxes of a page that are reading regions, in the frame of the page box, or why the
/// page uses none. `width` and `height` are the page's size; `widgets` the boxes of its form
/// fields. A box is a rectangle, four lines that meet, or a cell of a grid of one row or one
/// column; a cell of a grid of two rows and two columns or more is a table's. A box that
/// covers most of the page (its frame) or holds a form field is no region.
pub(crate) fn boxes(
    grids: Result<&Grids, Unsearched>,
    width: f32,
    height: f32,
    widgets: &[[f32; 4]],
    inked: &Inked,
) -> Result<Vec<[f32; 4]>, String> {
    let grids = match grids {
        Ok(g) => g,
        Err(Unsearched::TooMany) => {
            return Err(format!(
                "the page draws more than {MAX_SEGMENTS} straight lines"
            ));
        }
        Err(Unsearched::MeetTooOften) => return Err("its lines meet too often".into()),
    };
    let too_much = || "its boxes take too many steps to find".to_string();
    let mut work = 0usize;
    let mut out: Vec<[f32; 4]> = Vec::new();
    let page = width.max(0.0) * height.max(0.0);
    for (gh, gv) in &grids.groups {
        let h: Vec<Seg> = gh.iter().map(|&j| grids.hs[j]).collect();
        let v: Vec<Seg> = gv.iter().map(|&k| grids.vs[k]).collect();
        let mut xs: Vec<f32> = v.iter().map(|s| s.at).collect();
        xs.push(h.iter().map(|s| s.from).fold(f32::MAX, f32::min));
        xs.push(h.iter().map(|s| s.to).fold(f32::MIN, f32::max));
        let mut ys: Vec<f32> = h.iter().map(|s| s.at).collect();
        ys.push(v.iter().map(|s| s.from).fold(f32::MAX, f32::min));
        ys.push(v.iter().map(|s| s.to).fold(f32::MIN, f32::max));
        let xs = clustered(xs);
        let ys = clustered(ys);
        let (cols, rows) = (xs.len().saturating_sub(1), ys.len().saturating_sub(1));
        if cols == 0 || rows == 0 || (cols >= 2 && rows >= 2) {
            continue;
        }
        // The segments on each boundary, sorted along it.
        let on = |edges: &[f32], segs: &[Seg]| -> Vec<Vec<Seg>> {
            let mut out: Vec<Vec<Seg>> = vec![Vec::new(); edges.len()];
            for s in segs {
                let k = edges.partition_point(|e| *e < s.at);
                let near = [k.checked_sub(1), Some(k)]
                    .into_iter()
                    .flatten()
                    .filter(|&i| i < edges.len() && (edges[i] - s.at).abs() <= SNAP)
                    .min_by(|&a, &b| (edges[a] - s.at).abs().total_cmp(&(edges[b] - s.at).abs()));
                if let Some(i) = near {
                    out[i].push(*s);
                }
            }
            for list in &mut out {
                list.sort_by(|a, b| a.from.total_cmp(&b.from));
            }
            out
        };
        let on_x = on(&xs, &v);
        let on_y = on(&ys, &h);
        // Whether lines cover most of the stretch lo..hi of a boundary.
        // Strokes that overlap count once: the end covered so far moves along.
        let drawn = |segs: &[Seg], lo: f32, hi: f32, work: &mut usize| -> bool {
            let (mut len, mut end) = (0.0f32, lo);
            for s in segs {
                *work += 1;
                if s.from >= hi {
                    break;
                }
                len += (s.to.min(hi) - s.from.max(end)).max(0.0);
                end = end.max(s.to);
            }
            len >= DRAWN_SIDE * (hi - lo)
        };
        for r in 0..rows {
            for c in 0..cols {
                let (x0, x1, y0, y1) = (xs[c], xs[c + 1], ys[r], ys[r + 1]);
                let closed = drawn(&on_y[r], x0, x1, &mut work)
                    && drawn(&on_y[r + 1], x0, x1, &mut work)
                    && drawn(&on_x[c], y0, y1, &mut work)
                    && drawn(&on_x[c + 1], y0, y1, &mut work);
                if work > BOX_WORK {
                    return Err(too_much());
                }
                // A box without text in it is no region and counts toward no bound.
                let b = [x0, y0, x1, y1];
                if !closed || (x1 - x0) * (y1 - y0) > PAGE_SHARE * page || !inked.may_hold(b) {
                    continue;
                }
                if out.len() == MAX_BOXES {
                    return Err(format!("the page draws more than {MAX_BOXES} boxes"));
                }
                out.push(b);
            }
        }
    }
    // A box drawn twice a little apart is one.
    out.sort_by(|a, b| {
        a[0].total_cmp(&b[0])
            .then(a[1].total_cmp(&b[1]))
            .then(a[2].total_cmp(&b[2]))
    });
    out.dedup_by(|b, a| (0..4).all(|k| (a[k] - b[k]).abs() <= SNAP));
    if widgets.is_empty() || out.is_empty() {
        return Ok(out);
    }
    // A box that holds a form field is part of the form.
    let mut middles: Vec<(f32, f32)> = widgets
        .iter()
        .map(|w| ((w[0] + w[2]) / 2.0, (w[1] + w[3]) / 2.0))
        .collect();
    middles.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut kept = Vec::with_capacity(out.len());
    for b in out {
        let lo = middles.partition_point(|m| m.0 < b[0]);
        let mut holds = false;
        for m in &middles[lo..] {
            work += 1;
            if m.0 > b[2] {
                break;
            }
            if m.1 > b[1] && m.1 < b[3] {
                holds = true;
                break;
            }
        }
        if work > BOX_WORK {
            return Err(too_much());
        }
        if !holds {
            kept.push(b);
        }
    }
    Ok(kept)
}

/// Cells of the map of where a page has text, along each side.
const INKED_SIDE: usize = 64;

/// Where a page has text, coarsely: the number of glyphs in each cell of a grid over the
/// page, summed so that any box asks in four steps whether a glyph may lie in it.
pub(crate) struct Inked {
    step: (f32, f32),
    /// Summed counts: entry (r, c) holds the glyphs in cells below row r and left of column
    /// c, with one row and one column of zeros first.
    sums: Vec<u64>,
}

impl Inked {
    /// The map of `points` (glyph places) on a page of `width` by `height`.
    pub(crate) fn new(width: f32, height: f32, points: impl Iterator<Item = (f32, f32)>) -> Inked {
        let n = INKED_SIDE;
        let step = (
            (width / n as f32).max(f32::MIN_POSITIVE),
            (height / n as f32).max(f32::MIN_POSITIVE),
        );
        let mut counts = vec![0u32; n * n];
        for (x, y) in points {
            if let (Some(c), Some(r)) = (cell_of(x, step.0, n), cell_of(y, step.1, n)) {
                counts[r * n + c] = counts[r * n + c].saturating_add(1);
            }
        }
        let mut sums = vec![0u64; (n + 1) * (n + 1)];
        for r in 0..n {
            for c in 0..n {
                sums[(r + 1) * (n + 1) + c + 1] = u64::from(counts[r * n + c])
                    + sums[r * (n + 1) + c + 1]
                    + sums[(r + 1) * (n + 1) + c]
                    - sums[r * (n + 1) + c];
            }
        }
        Inked { step, sums }
    }

    /// Whether a glyph may lie in `b`: one lies in a cell the box reaches into.
    pub(crate) fn may_hold(&self, b: [f32; 4]) -> bool {
        let n = INKED_SIDE;
        let clamp = |v: f32, step: f32| ((v / step).floor().max(0.0) as usize).min(n - 1);
        if !(b[0] <= b[2] && b[1] <= b[3]) {
            return false;
        }
        let (c0, c1) = (clamp(b[0], self.step.0), clamp(b[2], self.step.0) + 1);
        let (r0, r1) = (clamp(b[1], self.step.1), clamp(b[3], self.step.1) + 1);
        let at = |r: usize, c: usize| self.sums[r * (n + 1) + c];
        at(r1, c1) + at(r0, c0) > at(r0, c1) + at(r1, c0)
    }
}

/// The cell of a value on a side of `n` cells of `step`, `None` off the page.
fn cell_of(v: f32, step: f32, n: usize) -> Option<usize> {
    let k = (v / step).floor();
    (k >= 0.0 && k < n as f32).then_some(k as usize)
}

/// Boxes found by the points they hold: a grid over their extent, each cell listing the boxes
/// that reach into it.
pub(crate) struct Index<'b> {
    boxes: &'b [[f32; 4]],
    lo: (f32, f32),
    step: (f32, f32),
    cells: Vec<Vec<u32>>,
    /// Steps taken by lookups so far.
    pub work: usize,
}

impl<'b> Index<'b> {
    /// The index, or `None` when the boxes would take too many places in it.
    pub(crate) fn new(boxes: &'b [[f32; 4]]) -> Option<Index<'b>> {
        let x0 = boxes.iter().map(|b| b[0]).fold(f32::MAX, f32::min);
        let y0 = boxes.iter().map(|b| b[1]).fold(f32::MAX, f32::min);
        let x1 = boxes.iter().map(|b| b[2]).fold(f32::MIN, f32::max);
        let y1 = boxes.iter().map(|b| b[3]).fold(f32::MIN, f32::max);
        let side = INDEX_SIDE as f32;
        let step = (
            ((x1 - x0) / side).max(f32::MIN_POSITIVE),
            ((y1 - y0) / side).max(f32::MIN_POSITIVE),
        );
        let mut index = Index {
            boxes,
            lo: (x0, y0),
            step,
            cells: vec![Vec::new(); INDEX_SIDE * INDEX_SIDE],
            work: 0,
        };
        let mut entries = 0usize;
        for (i, b) in boxes.iter().enumerate() {
            let (c0, r0) = index.cell(b[0], b[1]);
            let (c1, r1) = index.cell(b[2], b[3]);
            entries += (c1 - c0 + 1) * (r1 - r0 + 1);
            if entries > INDEX_ENTRIES {
                return None;
            }
            for r in r0..=r1 {
                for c in c0..=c1 {
                    index.cells[r * INDEX_SIDE + c].push(i as u32);
                }
            }
        }
        Some(index)
    }

    fn cell(&self, x: f32, y: f32) -> (usize, usize) {
        let at = |v: f32, lo: f32, step: f32| {
            let k = ((v - lo) / step).floor();
            if k.is_nan() || k < 0.0 {
                0
            } else {
                (k as usize).min(INDEX_SIDE - 1)
            }
        };
        (at(x, self.lo.0, self.step.0), at(y, self.lo.1, self.step.1))
    }

    /// The smallest box that holds the point, by its index.
    pub(crate) fn innermost(&mut self, x: f32, y: f32) -> Option<u32> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let (c, r) = self.cell(x, y);
        let mut best: Option<(f32, u32)> = None;
        for &i in &self.cells[r * INDEX_SIDE + c] {
            self.work += 1;
            let b = self.boxes[i as usize];
            if x > b[0] && x < b[2] && y > b[1] && y < b[3] {
                let area = (b[2] - b[0]) * (b[3] - b[1]);
                if best.is_none_or(|(a, _)| area < a) {
                    best = Some((area, i));
                }
            }
        }
        best.map(|(_, i)| i)
    }

    /// Hands `each` every box that holds the point.
    pub(crate) fn holders(&mut self, x: f32, y: f32, mut each: impl FnMut(u32)) {
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        let (c, r) = self.cell(x, y);
        for &i in &self.cells[r * INDEX_SIDE + c] {
            self.work += 1;
            let b = self.boxes[i as usize];
            if x > b[0] && x < b[2] && y > b[1] && y < b[3] {
                each(i);
            }
        }
    }
}

/// Two baselines this many font sizes apart or less are one: lines side by side that share
/// most of their baselines are rows, not two blocks.
const BASELINE_NEAR: f32 = 0.15;

/// Lines of one block follow each other at most this many font sizes apart, on median.
const BLOCK_PITCH: f32 = 4.0;

/// A baseline shift between two cells of a row that their size difference explains (top,
/// centre or baseline alignment of the two sizes) is at most this share of the difference,
/// plus [`BASELINE_NEAR`] of the smaller size.
const ALIGN_SHIFT: f32 = 0.8;

/// One part of a line: its baseline, its font size and whether it has ink (white space
/// only has no size to align with the cell beside it).
pub(crate) type Part = (f32, f32, bool);

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
pub(crate) fn offset_baselines(left: &[Vec<Part>], right: &[Vec<Part>], size: f32) -> bool {
    if left.len() < 2 || right.len() < 2 {
        return false;
    }
    // The first part of a line: its highest baseline, with its size.
    let head = |l: &Vec<Part>| -> Part {
        l.iter().copied().fold(
            (f32::MIN, size, false),
            |a, p| if p.0 > a.0 { p } else { a },
        )
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

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rule {
        Rule {
            x0,
            y0,
            x1,
            y1,
            kind: RuleKind::Rect,
        }
    }

    fn line(x0: f32, y0: f32, x1: f32, y1: f32) -> Rule {
        Rule {
            x0,
            y0,
            x1,
            y1,
            kind: RuleKind::Line,
        }
    }

    #[test]
    fn boxes_from_rectangles_lines_and_one_row_grids() {
        let rules = [
            // A rectangle, drawn twice a little apart.
            rect(40.0, 600.0, 240.0, 720.0),
            rect(40.7, 600.5, 240.4, 719.6),
            // Four lines that meet.
            line(300.0, 600.0, 500.0, 600.0),
            line(300.0, 700.0, 500.0, 700.0),
            line(300.0, 600.0, 300.0, 700.0),
            line(500.0, 600.0, 500.0, 700.0),
            // A box split in two by one rule: a grid of one row.
            line(50.0, 100.0, 450.0, 100.0),
            line(50.0, 150.0, 450.0, 150.0),
            line(50.0, 100.0, 50.0, 150.0),
            line(250.0, 100.0, 250.0, 150.0),
            line(450.0, 100.0, 450.0, 150.0),
            // A grid of two by two is a table's.
            line(50.0, 300.0, 250.0, 300.0),
            line(50.0, 350.0, 250.0, 350.0),
            line(50.0, 400.0, 250.0, 400.0),
            line(50.0, 300.0, 50.0, 400.0),
            line(150.0, 300.0, 150.0, 400.0),
            line(250.0, 300.0, 250.0, 400.0),
            // The page's frame.
            rect(1.0, 1.0, 594.0, 841.0),
            // An L of two lines is no box.
            line(300.0, 300.0, 400.0, 300.0),
            line(300.0, 300.0, 300.0, 400.0),
        ];
        let inked = Inked::new(
            595.0,
            842.0,
            [
                (100.0, 650.0),
                (100.0, 120.0),
                (300.0, 120.0),
                (350.0, 650.0),
            ]
            .into_iter(),
        );
        let found = grids(&rules);
        let mut got =
            boxes(found.as_ref().map_err(|e| *e), 595.0, 842.0, &[], &inked).expect("boxes");
        got.iter_mut()
            .for_each(|b| b.iter_mut().for_each(|v| *v = v.round()));
        assert_eq!(
            got,
            [
                [40.0, 600.0, 240.0, 720.0],
                [50.0, 100.0, 250.0, 150.0],
                [250.0, 100.0, 450.0, 150.0],
                [300.0, 600.0, 500.0, 700.0],
            ]
        );
        // A box that holds a form field is not a region.
        let fields = [[310.0, 610.0, 400.0, 630.0]];
        let got = boxes(
            found.as_ref().map_err(|e| *e),
            595.0,
            842.0,
            &fields,
            &inked,
        );
        assert_eq!(got.expect("boxes").len(), 3);
        // A box with no text in it is no region.
        let bare = Inked::new(595.0, 842.0, [(100.0, 650.0)].into_iter());
        let got = boxes(found.as_ref().map_err(|e| *e), 595.0, 842.0, &[], &bare);
        let got = got.expect("boxes");
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0][0].round(), 40.0);
    }

    #[test]
    fn the_innermost_box_holds_a_point() {
        let list = [
            [0.0, 0.0, 100.0, 100.0],
            [10.0, 10.0, 50.0, 50.0],
            [60.0, 60.0, 90.0, 90.0],
        ];
        let mut index = Index::new(&list).expect("index");
        assert_eq!(index.innermost(20.0, 20.0), Some(1));
        assert_eq!(index.innermost(55.0, 20.0), Some(0));
        assert_eq!(index.innermost(70.0, 70.0), Some(2));
        assert_eq!(index.innermost(200.0, 20.0), None);
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
        // The same shift between lines of one size is not explained: two blocks.
        assert!(offset_baselines(
            &at(&labels, 10.0),
            &at(&values, 10.0),
            10.0
        ));
    }
}
