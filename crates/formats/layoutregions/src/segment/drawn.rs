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
///
/// Points, not a share of the text size, by a measure of Task 31a: a rule has no text size of
/// its own, and with 0.2 of the page's median text size (2 points at 10 points) a page at
/// half or twice its size kept its grids, but pages set in small text lost joins that their
/// drawing needs (a ruled table whose rules miss each other by more than 1.4 points, rules
/// drawn dash by dash 2 points apart), and today's readers measure the same lines in points.
/// The gap between two dashes and the miss of two line ends are measures of the drawing, not
/// of the text: a snap that follows the scale of a page needs a measure taken from its rules.
///
/// Measured again in Task 31c with the snap handed in from the file's body size (0.2 sizes)
/// and the runs in the order of their places: the half-scale copy of a page kept the grids of
/// the page, but small-text pages lost the joins of their grids at their own size (a ruled
/// table of 4 by 7 read as a box of 2 by 1), and nine metamorphic pin counts on four pages
/// went up.
/// With the page's median text size 43 went up; with the larger of 2 points and 0.2 sizes a
/// date label joined its year at one and a half times the size.
pub const SNAP: f32 = 2.0;

/// A grid or region that covers more of the page than this is the page's frame.
pub const PAGE_SHARE: f32 = 0.6;

/// Straight segments the grid search looks at on one page; past them it gives up on the page
/// and says so.
pub const MAX_SEGMENTS: usize = 20_000;

/// Meetings of an upright and a flat segment the grid search follows on one page (a flat
/// segment that reaches across the place of an upright one, in its height); past them it
/// gives up on the page and says so.
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

/// Lines by their place and then their start (as [`merged`] gives them), to visit those
/// near a place that reach into a stretch without a look at every line near the place: a
/// column of some thousand boxes, or a row of as many small grids, has that many lines at
/// one place.
#[derive(Debug, Clone)]
pub struct LinesAt<'a> {
    segs: &'a [Line],
    /// For each line the farthest end of the lines at its own place up to it: the lines
    /// before a line that all end before a stretch are passed over at once. `None` when a
    /// line has a place, a start or an end that is no finite number: such lines have no
    /// order to search in, and every line is looked at.
    ends: Option<Vec<f32>>,
}

impl<'a> LinesAt<'a> {
    /// `segs` by place and then start; one step of work a line.
    pub fn new(segs: &'a [Line]) -> LinesAt<'a> {
        crate::work::add(segs.len() as u64);
        let mut ends: Vec<f32> = Vec::with_capacity(segs.len());
        for (k, s) in segs.iter().enumerate() {
            if !(s.at.is_finite() && s.from.is_finite() && s.to.is_finite()) {
                return LinesAt { segs, ends: None };
            }
            let end = match k.checked_sub(1) {
                Some(j) if segs[j].at.to_bits() == s.at.to_bits() => ends[j].max(s.to),
                _ => s.to,
            };
            ends.push(end);
        }
        LinesAt {
            segs,
            ends: Some(ends),
        }
    }

    /// Whether every line has a finite place, start and end, so the lines stand in an order
    /// to search in.
    pub fn ordered(&self) -> bool {
        self.ends.is_some()
    }

    /// Calls `f` with each line (its index and the line) whose place lies in `low..=high`
    /// and that reaches into the stretch `from..to`: it ends after `from` and starts before
    /// `to`. The lines come in no stated order. Returns the steps it took, which are counted
    /// as work too: one for each place with lines in `low..=high` and one for each line
    /// looked at.
    pub fn reaching(
        &self,
        (low, high): (f32, f32),
        (from, to): (f32, f32),
        mut f: impl FnMut(usize, &Line),
    ) -> u64 {
        let segs = self.segs;
        let mut steps = 0u64;
        match &self.ends {
            Some(ends) => {
                let mut at = segs.partition_point(|s| s.at < low);
                while at < segs.len() && segs[at].at <= high {
                    // The lines at one place, by their start: those that start before the
                    // end of the stretch, back to the last one no line before which
                    // reaches into it.
                    let place = segs[at].at;
                    let end = at
                        + segs[at..]
                            .partition_point(|s| s.at.total_cmp(&place).is_le())
                            .max(1);
                    let mut k = at + segs[at..end].partition_point(|s| s.from < to);
                    steps += 1;
                    while k > at && ends[k - 1] > from {
                        k -= 1;
                        steps += 1;
                        if segs[k].to > from {
                            f(k, &segs[k]);
                        }
                    }
                    at = end;
                }
            }
            None => {
                for (k, s) in segs.iter().enumerate() {
                    steps += 1;
                    if s.at >= low && s.at <= high && s.to > from && s.from < to {
                        f(k, s);
                    }
                }
            }
        }
        crate::work::add(steps);
        steps
    }
}

/// Why the lines of a page were not searched for grids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unsearched {
    /// More straight segments than [`MAX_SEGMENTS`]: `lines` is how many the page draws (a
    /// rectangle counts its four sides).
    TooMany { lines: usize },
    /// More meetings of segments than [`GRID_WORK`].
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
    drawn_on(page, grids(&page.rules), inked)
}

/// [`drawn`] on the grids of `page.rules` a host already found ([`grids`]), so the lines are
/// searched once per page.
pub fn drawn_on(
    page: &PageIn,
    grids: Result<Grids, Unsearched>,
    inked: &dyn Fn(Bounds) -> bool,
) -> DrawnStructure {
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
    // A walk across the page, from left to right over the vertical lines, with the horizontal
    // lines that reach across the place of the walk at hand, by their height: each vertical
    // line then looks only at the horizontal lines it meets, not at every one in its
    // height. So the steps are the meetings, and a page of many small grids side by side,
    // whose lines share their heights, costs about its lines.
    let mut steps = 0usize;
    let by = |key: fn(&Line) -> f32, of: &[Line]| {
        let mut ids: Vec<usize> = (0..of.len()).filter(|&i| !key(&of[i]).is_nan()).collect();
        ids.sort_by(|&a, &b| key(&of[a]).total_cmp(&key(&of[b])).then(a.cmp(&b)));
        ids
    };
    // A line with a place or an end that is no number meets none.
    let whole = |l: &Line| !(l.at.is_nan() || l.from.is_nan() || l.to.is_nan());
    let (starts, ends, walk) = (by(|h| h.from, &hs), by(|h| h.to, &hs), by(|v| v.at, &vs));
    work::add(2 * (starts.len() + ends.len() + walk.len()) as u64);
    // The horizontal lines at hand, each as the bits of its height (in the order of
    // `f32::total_cmp`, with the two zeros as one) and its number.
    let key = |at: f32| {
        let bits = (at + 0.0).to_bits();
        if bits >> 31 == 0 {
            bits | 1 << 31
        } else {
            !bits
        }
    };
    let mut at_hand: std::collections::BTreeSet<(u32, usize)> = std::collections::BTreeSet::new();
    let (mut next_start, mut next_end) = (0, 0);
    for &k in &walk {
        let v = vs[k];
        while let Some(&j) = starts
            .get(next_start)
            .filter(|&&j| hs[j].from - SNAP <= v.at)
        {
            next_start += 1;
            if whole(&hs[j]) {
                at_hand.insert((key(hs[j].at), j));
            }
        }
        while let Some(&j) = ends.get(next_end).filter(|&&j| hs[j].to + SNAP < v.at) {
            next_end += 1;
            at_hand.remove(&(key(hs[j].at), j));
        }
        let (low, high) = (v.from - SNAP, v.to + SNAP);
        if !whole(&v) || low > high {
            continue;
        }
        for &(_, j) in at_hand.range((key(low), 0)..=(key(high), usize::MAX)) {
            steps += 1;
            if steps > GRID_WORK {
                work::add(steps as u64);
                return Err(Unsearched::Work);
            }
            let (a, b) = (find(&mut parent, j), find(&mut parent, n + k));
            parent[a] = b;
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
    // In the order of each group's first line across: the map gives its groups in an order
    // that differs from run to run, and the regions of a page follow the order of the grids.
    let mut groups: Vec<(Vec<usize>, Vec<usize>)> = groups
        .into_values()
        .filter(|(h, v)| h.len() >= 2 && v.len() >= 2)
        .collect();
    groups.sort_unstable_by_key(|(h, _)| h[0]);
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
    /// Fix round 1 of Task 37, M4: the lines found by the search are the lines a look at
    /// every line finds (the look the title bars, the strips and the cover of a gap between
    /// boxes took before Task 37): lines apart, lines that overlap and one long line among
    /// short ones, at places near each other, asked for 8,000 stretches.
    #[test]
    fn the_lines_that_reach_a_stretch_are_those_a_look_at_every_line_finds() {
        let mut seed = 11u64;
        let mut roll = |n: u32| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((seed >> 33) % u64::from(n)) as f32
        };
        let mut some = 0;
        for round in 0..40 {
            let mut segs: Vec<Line> = (0..80)
                .map(|_| {
                    let from = 5.0 * roll(40);
                    Line {
                        at: 0.5 * roll(8),
                        from,
                        to: from + 5.0 * (1.0 + roll(4)),
                    }
                })
                .collect();
            if round % 2 == 0 {
                segs.push(Line {
                    at: 1.0,
                    from: 0.0,
                    to: 150.0,
                });
            }
            segs.sort_by(|a, b| a.at.total_cmp(&b.at).then(a.from.total_cmp(&b.from)));
            let lines = LinesAt::new(&segs);
            assert!(lines.ordered());
            for _ in 0..200 {
                let low = 0.5 * roll(8) - 0.25 * roll(3);
                let high = low + 0.25 * roll(5);
                let from = 5.0 * roll(40);
                let to = from + 5.0 * roll(6);
                let want: Vec<usize> = (0..segs.len())
                    .filter(|&k| {
                        let s = &segs[k];
                        s.at >= low && s.at <= high && s.to > from && s.from < to
                    })
                    .collect();
                let mut got = Vec::new();
                lines.reaching((low, high), (from, to), |k, _| got.push(k));
                got.sort_unstable();
                assert_eq!(got, want, "{low}..={high}, {from}..{to}");
                some += usize::from(!want.is_empty());
            }
        }
        assert!(some > 1_000, "{some} stretches with lines");
    }

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

    /// Rectangles drawn side by side that share their sides are one group of lines: their
    /// shared sides are one line, and the group is a grid of one row with a cell per
    /// rectangle. Rectangles a gap apart stay groups of their own.
    /// The groups stand in the order of their first line across, on every search: a page of
    /// many grids gives the same groups in the same order each time (a map's own order differs
    /// from one map to the next).
    #[test]
    fn the_groups_of_a_page_stand_in_one_order_on_every_search() {
        let rules: Vec<RuleIn> = (0..24)
            .map(|k| {
                let (x, y) = (40.0 + 130.0 * (k % 4) as f32, 60.0 + 110.0 * (k / 4) as f32);
                rect(x, y, x + 90.0, y + 70.0)
            })
            .collect();
        let first = grids(&rules).expect("searched");
        assert_eq!(first.groups.len(), 24);
        let firsts: Vec<usize> = first.groups.iter().map(|g| g.0[0]).collect();
        assert!(firsts.is_sorted(), "{firsts:?}");
        for _ in 0..8 {
            assert_eq!(grids(&rules).expect("searched"), first);
        }
    }

    #[test]
    fn abutting_rectangles_are_one_group() {
        let rules = vec![
            rect(50.0, 600.0, 150.0, 620.0),
            rect(150.0, 600.0, 250.0, 620.0),
            rect(250.0, 600.0, 350.0, 620.0),
            rect(50.0, 500.0, 150.0, 520.0),
            rect(156.0, 500.0, 256.0, 520.0),
        ];
        let g = grids(&rules).expect("searched");
        assert_eq!(g.groups.len(), 3, "{g:?}");
        let mut frames: Vec<(Bounds, u32, u32)> = g
            .groups
            .iter()
            .filter_map(|group| grid_frame(&g, group))
            .collect();
        assert_eq!(frames.len(), 1, "only the abutting row is a grid");
        let (b, rows, cols) = frames.remove(0);
        assert_eq!((rows, cols), (1, 3));
        assert_eq!(
            [b.x0, b.y0, b.x1, b.y1].map(f32::round),
            [50.0, 600.0, 350.0, 620.0]
        );
    }
}
