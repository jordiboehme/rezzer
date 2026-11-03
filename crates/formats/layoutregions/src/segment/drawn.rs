// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Drawn lines that meet, and the boxes they draw. A grid of at least two rows and two
//! columns is a table's; a box that stands alone, or a cell of a grid of one row or one
//! column, is a reading region: its text is read whole, before or after the text around it.
//!
//! [`drawn`] finds both once per page, and every finder that looks at drawn lines uses what
//! it found.

use std::collections::HashMap;

use crate::input::{PageIn, RuleIn};
use crate::region::Bounds;
use crate::work;

/// Lines this close are one line, and line ends this close meet, in points.
pub const SNAP: f32 = 2.0;

/// A grid or region that covers more of the page than this is the page's frame.
pub const PAGE_SHARE: f32 = 0.6;

/// Straight segments the grid search looks at on one page; past them it gives up on the page
/// and says so.
pub const MAX_SEGMENTS: usize = 20_000;

/// Pairs of segments the grid search tests on one page; past them it gives up on the page and
/// says so.
pub const GRID_WORK: usize = 4_000_000;

/// Joined lines a group of segments keeps open for joining at once; past them the oldest is
/// closed. Only a page of many long overlapping lines at slightly different places has more.
const OPEN_LINES: usize = 32;

/// Boxes one page may use as reading regions; past them it uses none and says so.
pub const MAX_BOXES: usize = 4_096;

/// Steps the box search may take on one page (sides checked, widgets looked at, fragments
/// looked up); past them the page uses no boxes and says so.
pub const BOX_WORK: usize = 4_000_000;

/// A box side is drawn when lines cover this share of it.
const DRAWN_SIDE: f32 = 0.9;

#[cfg(debug_assertions)]
thread_local! {
    static SEARCHES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The grid searches ([`grids`], also run by [`drawn`]) done on this thread so far; `None` in
/// a release build, which does not count them. A test reads it before and after a call, to
/// check that a page's lines are searched once.
pub fn grid_searches() -> Option<u64> {
    #[cfg(debug_assertions)]
    {
        Some(SEARCHES.with(std::cell::Cell::get))
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

/// A straight horizontal (`at` is y, `from`..`to` along x) or vertical line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line {
    pub at: f32,
    pub from: f32,
    pub to: f32,
}

/// Why the lines of a page were not searched for grids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unsearched {
    /// More straight segments than [`MAX_SEGMENTS`]: `lines` is how many the page draws (a
    /// rectangle counts its four sides).
    TooMany { lines: usize },
    /// More pairs of segments to test than [`GRID_WORK`].
    Work,
}

impl Unsearched {
    /// Why, in words.
    pub fn why(self) -> String {
        match self {
            Unsearched::TooMany { .. } => {
                format!("the page draws more than {MAX_SEGMENTS} straight lines")
            }
            Unsearched::Work => "its lines meet too often".into(),
        }
    }
}

/// The joined lines of a page and the groups of them that meet: indices into `hs` and `vs`,
/// each group with at least two of each.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Grids {
    pub hs: Vec<Line>,
    pub vs: Vec<Line>,
    pub groups: Vec<(Vec<usize>, Vec<usize>)>,
}

/// What the drawn lines of a page make: its grids, and the boxes its text is read in as
/// reading regions, in the frame of the page.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawnStructure {
    /// The grids, or why the lines were not searched.
    pub grids: Result<Grids, Unsearched>,
    /// The boxes that are reading regions; empty when `boxes_note` says why none is used.
    pub boxes: Vec<Bounds>,
    /// Why the page uses no boxes as reading regions, when a bound stopped the search or the
    /// lines were not searched.
    pub boxes_note: Option<String>,
}

/// The grids and boxes of `page`, from its rules; its widget boxes are its form fields.
/// `inked` tells whether text may lie in a box: a box without text in it is no region.
///
/// ```
/// use layoutregions::segment::drawn::drawn;
/// use layoutregions::{Bounds, PageIn, RuleIn};
///
/// let rect = RuleIn {
///     x0: 40.0,
///     y0: 600.0,
///     x1: 240.0,
///     y1: 720.0,
///     thick: 0.5,
///     dashed: false,
///     rect: true,
/// };
/// let page = PageIn {
///     page: 1,
///     size: [595.0, 842.0],
///     turn: 0,
///     texts: Vec::new(),
///     rules: vec![rect],
///     boxes: Vec::new(),
///     pictures: Vec::new(),
///     codes: Vec::new(),
///     given: Vec::new(),
/// };
/// let found = drawn(&page, &|_: Bounds| true);
/// assert_eq!(found.boxes.len(), 1);
/// assert_eq!(found.grids.map(|g| g.groups.len()), Ok(1));
/// ```
pub fn drawn(page: &PageIn, inked: &dyn Fn(Bounds) -> bool) -> DrawnStructure {
    let grids = grids(&page.rules);
    let (boxes, boxes_note) = match &grids {
        Ok(g) => {
            let widgets: Vec<[f32; 4]> = page
                .boxes
                .iter()
                .filter(|b| b.widget)
                .map(|b| [b.bounds.x0, b.bounds.y0, b.bounds.x1, b.bounds.y1])
                .collect();
            match boxes(g, page.size[0], page.size[1], &widgets, inked) {
                Ok(b) => (b, None),
                Err(why) => (Vec::new(), Some(why)),
            }
        }
        Err(e) => (Vec::new(), Some(e.why())),
    };
    DrawnStructure {
        grids,
        boxes,
        boxes_note,
    }
}

/// Collinear segments that touch become one. Segments are grouped by where they lie: a
/// segment joins the group when it lies within [`SNAP`] of the group's mean place, so lines
/// in a dense run (bars of a barcode, hatching) never chain far apart. Within a group, a
/// piece joins a line that it overlaps or touches within [`SNAP`] and that lies within
/// [`SNAP`] of it; the joined line lies at the mean of its pieces. Two lines a point apart
/// that never meet along their length stay apart. The result is sorted by place, then start.
pub fn merged(mut segs: Vec<Line>) -> Vec<Line> {
    segs.sort_by(|a, b| a.at.total_cmp(&b.at));
    let mut out: Vec<Line> = Vec::with_capacity(segs.len());
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
        let mut open: Vec<(Line, f64, u32)> = Vec::new();
        let close = |(mut s, sum, n): (Line, f64, u32), out: &mut Vec<Line>| {
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
pub fn clustered(mut values: Vec<f32>) -> Vec<f32> {
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

/// Whether the group `(gh, gv)` of `g` holds rules inside its frame: three rules or more of
/// one direction (pieces of one line count once). A frame alone (a box, the sides of a figure
/// or of a shaded band) has two of each.
pub fn inner_ruled(g: &Grids, (gh, gv): &(Vec<usize>, Vec<usize>)) -> bool {
    let distinct = |mut at: Vec<f32>| {
        at.sort_by(f32::total_cmp);
        at.dedup_by(|b, a| *b - *a <= SNAP);
        at.len()
    };
    distinct(gh.iter().map(|&j| g.hs[j].at).collect()) >= 3
        || distinct(gv.iter().map(|&k| g.vs[k].at).collect()) >= 3
}

/// The frame of the group `(gh, gv)` of `g` with its rows and columns, counted as the box
/// search counts them (its line places and the ends of its lines, [`clustered`]): the grid a
/// segment of [`crate::segment::Source::Grid`] stands for. `None` for a group of one cell (a
/// box) or none.
pub fn grid_frame(g: &Grids, (gh, gv): &(Vec<usize>, Vec<usize>)) -> Option<(Bounds, u32, u32)> {
    let mut xs: Vec<f32> = gv.iter().map(|&k| g.vs[k].at).collect();
    xs.extend(gh.iter().map(|&j| g.hs[j].from).reduce(f32::min));
    xs.extend(gh.iter().map(|&j| g.hs[j].to).reduce(f32::max));
    let mut ys: Vec<f32> = gh.iter().map(|&j| g.hs[j].at).collect();
    ys.extend(gv.iter().map(|&k| g.vs[k].from).reduce(f32::min));
    ys.extend(gv.iter().map(|&k| g.vs[k].to).reduce(f32::max));
    let (xs, ys) = (clustered(xs), clustered(ys));
    let (cols, rows) = (xs.len().saturating_sub(1), ys.len().saturating_sub(1));
    if cols == 0 || rows == 0 || cols * rows < 2 {
        return None;
    }
    let b = Bounds {
        x0: xs[0],
        y0: ys[0],
        x1: xs[cols],
        y1: ys[rows],
    };
    Some((b, rows as u32, cols as u32))
}

/// Horizontal and vertical lines that meet, in groups. A rectangle gives its four sides. A
/// page without vertical lines has its horizontal lines and no group.
///
/// [`drawn`] runs this once per page; a reader calls it on its own only for rules in another
/// frame than the page's.
pub fn grids(rules: &[RuleIn]) -> Result<Grids, Unsearched> {
    #[cfg(debug_assertions)]
    SEARCHES.with(|n| n.set(n.get() + 1));
    let flat = |r: &RuleIn| (r.y0 - r.y1).abs() <= SNAP / 2.0;
    // Counted before anything is built: a page may keep many more lines than are searched.
    let (mut h, mut v) = (0usize, 0usize);
    for r in rules {
        match r.rect {
            false if flat(r) => h += 1,
            false => v += 1,
            true => (h, v) = (h + 2, v + 2),
        }
    }
    work::add(rules.len() as u64);
    // Without horizontal lines there is no grid and no row of fields; horizontal lines alone
    // still part rows of fields, so they are kept with no group, unless they are more than
    // are searched (hatching): then there is nothing to say either.
    if h == 0 || (v == 0 && h > MAX_SEGMENTS) {
        return Ok(Grids::default());
    }
    if h + v > MAX_SEGMENTS {
        return Err(Unsearched::TooMany { lines: h + v });
    }
    let mut hs = Vec::with_capacity(h);
    let mut vs = Vec::with_capacity(v);
    for r in rules {
        let (x0, x1) = (r.x0.min(r.x1), r.x0.max(r.x1));
        let (y0, y1) = (r.y0.min(r.y1), r.y0.max(r.y1));
        match r.rect {
            false if flat(r) => hs.push(Line {
                at: (y0 + y1) / 2.0,
                from: x0,
                to: x1,
            }),
            false => vs.push(Line {
                at: (x0 + x1) / 2.0,
                from: y0,
                to: y1,
            }),
            true => {
                hs.push(Line {
                    at: y0,
                    from: x0,
                    to: x1,
                });
                hs.push(Line {
                    at: y1,
                    from: x0,
                    to: x1,
                });
                vs.push(Line {
                    at: x0,
                    from: y0,
                    to: y1,
                });
                vs.push(Line {
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
    let mut steps = 0usize;
    for (k, v) in vs.iter().enumerate() {
        let lo = hs.partition_point(|h| h.at < v.from - SNAP);
        for (j, h) in hs.iter().enumerate().skip(lo) {
            if h.at > v.to + SNAP {
                break;
            }
            steps += 1;
            if steps > GRID_WORK {
                work::add(steps as u64);
                return Err(Unsearched::Work);
            }
            if v.at >= h.from - SNAP && v.at <= h.to + SNAP {
                let (a, b) = (find(&mut parent, j), find(&mut parent, n + k));
                parent[a] = b;
            }
        }
    }
    work::add(steps as u64);
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

/// The boxes of a page that are reading regions, in the frame of the page, or why the page
/// uses none. `width` and `height` are the page's size; `widgets` the boxes of its form
/// fields. A box is a rectangle, four lines that meet, or a cell of a grid of one row or one
/// column; a cell of a grid of two rows and two columns or more is a table's. A box that
/// covers most of the page (its frame), holds no text (see `inked`) or holds a form field is
/// no region.
fn boxes(
    grids: &Grids,
    width: f32,
    height: f32,
    widgets: &[[f32; 4]],
    inked: &dyn Fn(Bounds) -> bool,
) -> Result<Vec<Bounds>, String> {
    let too_much = || "its boxes take too many steps to find".to_string();
    let mut steps = 0usize;
    let mut out: Vec<[f32; 4]> = Vec::new();
    let page = width.max(0.0) * height.max(0.0);
    for (gh, gv) in &grids.groups {
        let h: Vec<Line> = gh.iter().map(|&j| grids.hs[j]).collect();
        let v: Vec<Line> = gv.iter().map(|&k| grids.vs[k]).collect();
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
        let on = |edges: &[f32], segs: &[Line]| -> Vec<Vec<Line>> {
            let mut out: Vec<Vec<Line>> = vec![Vec::new(); edges.len()];
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
        let drawn = |segs: &[Line], lo: f32, hi: f32, steps: &mut usize| -> bool {
            let (mut len, mut end) = (0.0f32, lo);
            for s in segs {
                *steps += 1;
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
                let closed = drawn(&on_y[r], x0, x1, &mut steps)
                    && drawn(&on_y[r + 1], x0, x1, &mut steps)
                    && drawn(&on_x[c], y0, y1, &mut steps)
                    && drawn(&on_x[c + 1], y0, y1, &mut steps);
                if steps > BOX_WORK {
                    work::add(steps as u64);
                    return Err(too_much());
                }
                // A box without text in it is no region and counts toward no bound.
                if !closed
                    || (x1 - x0) * (y1 - y0) > PAGE_SHARE * page
                    || !inked(Bounds { x0, y0, x1, y1 })
                {
                    continue;
                }
                if out.len() == MAX_BOXES {
                    work::add(steps as u64);
                    return Err(format!("the page draws more than {MAX_BOXES} boxes"));
                }
                out.push([x0, y0, x1, y1]);
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
    let bounds = |b: [f32; 4]| Bounds {
        x0: b[0],
        y0: b[1],
        x1: b[2],
        y1: b[3],
    };
    if widgets.is_empty() || out.is_empty() {
        work::add(steps as u64);
        return Ok(out.into_iter().map(bounds).collect());
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
            steps += 1;
            if m.0 > b[2] {
                break;
            }
            if m.1 > b[1] && m.1 < b[3] {
                holds = true;
                break;
            }
        }
        if steps > BOX_WORK {
            work::add(steps as u64);
            return Err(too_much());
        }
        if !holds {
            kept.push(bounds(b));
        }
    }
    work::add(steps as u64);
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::BoxIn;

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> RuleIn {
        RuleIn {
            x0,
            y0,
            x1,
            y1,
            thick: 0.0,
            dashed: false,
            rect: true,
        }
    }

    fn line(x0: f32, y0: f32, x1: f32, y1: f32) -> RuleIn {
        RuleIn {
            rect: false,
            ..rect(x0, y0, x1, y1)
        }
    }

    fn seg(at: f32, from: f32, to: f32) -> Line {
        Line { at, from, to }
    }

    fn page(rules: Vec<RuleIn>, widgets: &[[f32; 4]]) -> PageIn {
        PageIn {
            page: 1,
            size: [595.0, 842.0],
            turn: 0,
            texts: Vec::new(),
            rules,
            boxes: widgets
                .iter()
                .map(|w| BoxIn {
                    bounds: Bounds {
                        x0: w[0],
                        y0: w[1],
                        x1: w[2],
                        y1: w[3],
                    },
                    filled: false,
                    stroked: false,
                    widget: true,
                })
                .collect(),
            pictures: Vec::new(),
            codes: Vec::new(),
            given: Vec::new(),
        }
    }

    /// Whether one of `points` lies in a box.
    fn ink(points: &[(f32, f32)]) -> impl Fn(Bounds) -> bool + '_ {
        |b: Bounds| {
            points
                .iter()
                .any(|&(x, y)| x >= b.x0 && x <= b.x1 && y >= b.y0 && y <= b.y1)
        }
    }

    fn rounded(boxes: &[Bounds]) -> Vec<[f32; 4]> {
        boxes
            .iter()
            .map(|b| [b.x0, b.y0, b.x1, b.y1].map(f32::round))
            .collect()
    }

    /// A page of more horizontal lines than are searched and no upright one (hatching) is not
    /// searched as before: no grid, no line kept, and no note that it was not searched.
    #[test]
    fn many_horizontal_lines_alone_are_no_grid_and_no_note() {
        let rules: Vec<RuleIn> = (0..=MAX_SEGMENTS)
            .map(|k| line(0.0, k as f32 * 0.04, 100.0, k as f32 * 0.04))
            .collect();
        let found = grids(&rules).expect("no note");
        assert!(found.hs.is_empty() && found.groups.is_empty());
        let searches = || grid_searches().expect("a test build counts grid searches");
        let before = searches();
        let found = drawn(&page(rules, &[]), &|_| true);
        assert_eq!(found.boxes_note, None);
        // `drawn` searches the lines once.
        assert_eq!(searches() - before, 1);
    }

    /// Lines past the bound are not searched, and the structure says why in the words of the
    /// bound; the page then uses no boxes.
    #[test]
    fn too_many_lines_are_not_searched() {
        let mut rules: Vec<RuleIn> = (0..MAX_SEGMENTS)
            .map(|k| line(0.0, k as f32 * 0.04, 100.0, k as f32 * 0.04))
            .collect();
        rules.push(line(50.0, 0.0, 50.0, 100.0));
        rules.push(rect(10.0, 10.0, 90.0, 90.0));
        assert_eq!(
            grids(&rules),
            Err(Unsearched::TooMany {
                lines: MAX_SEGMENTS + 5
            })
        );
        let found = drawn(&page(rules, &[]), &|_| true);
        assert!(found.boxes.is_empty());
        assert_eq!(
            found.boxes_note.as_deref(),
            Some("the page draws more than 20000 straight lines")
        );
        assert_eq!(Unsearched::Work.why(), "its lines meet too often");
    }

    #[test]
    fn boxes_from_rectangles_lines_and_one_row_grids() {
        let rules = vec![
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
        let text = [
            (100.0, 650.0),
            (100.0, 120.0),
            (300.0, 120.0),
            (350.0, 650.0),
        ];
        let found = drawn(&page(rules.clone(), &[]), &ink(&text));
        assert_eq!(found.boxes_note, None);
        assert_eq!(
            rounded(&found.boxes),
            [
                [40.0, 600.0, 240.0, 720.0],
                [50.0, 100.0, 250.0, 150.0],
                [250.0, 100.0, 450.0, 150.0],
                [300.0, 600.0, 500.0, 700.0],
            ]
        );
        // The grid of two by two holds rules inside its frame; the boxes do not.
        let grids = found.grids.expect("grids");
        let ruled = grids
            .groups
            .iter()
            .filter(|g| inner_ruled(&grids, g))
            .count();
        assert_eq!(ruled, 2, "the grid of one row and the grid of two by two");
        // A box that holds a form field is not a region.
        let fields = [[310.0, 610.0, 400.0, 630.0]];
        let found = drawn(&page(rules.clone(), &fields), &ink(&text));
        assert_eq!(found.boxes.len(), 3);
        // A box with no text in it is no region.
        let found = drawn(&page(rules, &[]), &ink(&[(100.0, 650.0)]));
        assert_eq!(found.boxes.len(), 1, "{:?}", found.boxes);
        assert_eq!(found.boxes[0].x0.round(), 40.0);
    }

    #[test]
    fn merged_keeps_near_collinear_segments() {
        // Two sides of a box drawn twice a little apart, and a column rule 1.5 points to
        // the right of them in three pieces lower down: the rule's pieces stay apart from
        // the box sides and from each other.
        let out = merged(vec![
            seg(411.5, 689.5, 739.8),
            seg(412.2, 689.5, 739.8),
            seg(413.3, 106.2, 261.5),
            seg(413.3, 481.0, 557.5),
            seg(413.3, 621.2, 672.2),
        ]);
        let mut got: Vec<(f32, f32, f32)> = out.iter().map(|s| (s.at, s.from, s.to)).collect();
        got.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.total_cmp(&b.0)));
        assert_eq!(got.len(), 4, "{got:?}");
        for (k, (from, to)) in [(106.2, 261.5), (481.0, 557.5), (621.2, 672.2)]
            .into_iter()
            .enumerate()
        {
            assert_eq!(got[k], (413.3, from, to), "{got:?}");
        }
        assert!((got[3].0 - 411.85).abs() < 0.01, "{got:?}");
        assert_eq!((got[3].1, got[3].2), (689.5, 739.8), "{got:?}");
        // Pieces that touch within the snap distance join; pieces of one line far apart
        // stay apart.
        let out = merged(vec![
            seg(100.0, 0.0, 10.0),
            seg(100.5, 11.5, 20.0),
            seg(100.0, 30.0, 40.0),
        ]);
        assert_eq!(out.len(), 2, "{out:?}");
        // Bars every 1.8 points and two column rules 40 points apart among them, over other
        // heights: the rules stay apart.
        let mut segs: Vec<Line> = (0..56)
            .map(|k| seg(400.0 + 1.8 * k as f32, 100.0, 140.0))
            .collect();
        segs.push(seg(410.0, 640.0, 700.0));
        segs.push(seg(450.0, 640.0, 700.0));
        let out = merged(segs);
        let rules: Vec<f32> = out
            .iter()
            .filter(|s| s.from == 640.0)
            .map(|s| s.at)
            .collect();
        assert_eq!(rules, [410.0, 450.0], "{out:?}");
    }

    #[test]
    fn values_closer_than_the_snap_are_one() {
        assert_eq!(
            clustered(vec![10.0, 3.0, 11.5, 4.0, 20.0]),
            [3.0, 10.0, 20.0]
        );
    }
}
