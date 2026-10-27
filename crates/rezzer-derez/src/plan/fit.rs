// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Fitting a plan to a grid of cells.

use std::collections::VecDeque;

use super::{Plan, Shape, ShapeKind};
use crate::grid::{Dir, Grid, Pos, Tile};
use crate::steer;

/// The free lane, in cells, between the border and the shapes.
pub const MARGIN: u16 = 2;
/// The width of a gap in a long line or an outline side.
pub const DOOR: u16 = 2;
/// The free cells every start needs ahead of it.
pub const START_RUN: u16 = 5;
/// The most of the inner area that walls and towers may cover.
pub const MAX_WALL_SHARE: f32 = 0.40;
/// The smallest grid, in cells.
pub const MIN_GRID: (u16, u16) = (30, 18);
/// The largest grid, in cells; a larger size is clamped to it.
pub const MAX_GRID: (u16, u16) = (120, 60);
/// The most shapes of one plan that are drawn; the rest are skipped.
pub const MAX_SHAPES: usize = 4096;

/// How far from its target a start is looked for before a run is carved free.
const START_SEARCH: u16 = 10;
/// The least Manhattan distance between two starts.
const START_GAP: u16 = 4;

/// A plan fitted to a grid.
#[derive(Debug, Clone, PartialEq)]
pub struct Fitted {
    pub grid: Grid,
    /// Start cell and direction of each program, the user program first.
    pub starts: [(Pos, Dir); 4],
}

/// Why a plan does not fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// Walls and towers cover too much of the grid.
    Crowded,
    /// The grid is below [`MIN_GRID`].
    TooSmall,
}

/// Maps `plan` to a grid of `w` by `h` cells (clamped to [`MAX_GRID`]).
///
/// The border is wall, and the first [`MAX_SHAPES`] shapes map into the area inside a free
/// lane of [`MARGIN`] cells. Long lines and outline sides get doors of [`DOOR`] cells. Every
/// free cell not connected to the lane becomes wall, so no area is sealed off. When walls and
/// towers cover more than [`MAX_WALL_SHARE`] of that area, before or after this sealing,
/// blocks are drawn as outlines; when they still do, the plan is [`Refused::Crowded`]. The
/// four starts are free, have [`START_RUN`] free cells ahead, lie on no other start's run and
/// are at least 4 cells apart (Manhattan).
pub fn fit(plan: &Plan, w: u16, h: u16) -> Result<Fitted, Refused> {
    if w < MIN_GRID.0 || h < MIN_GRID.1 {
        return Err(Refused::TooSmall);
    }
    let (w, h) = (w.min(MAX_GRID.0), h.min(MAX_GRID.1));
    let shapes = &plan.shapes[..plan.shapes.len().min(MAX_SHAPES)];
    // Without blocks, a hollow redraw is the same grid.
    let hollow = if shapes.iter().any(|s| s.kind == ShapeKind::Block) {
        &[false, true][..]
    } else {
        &[false][..]
    };
    let mut grid = hollow
        .iter()
        .find_map(|&hollow| {
            let mut grid = draw(shapes, w, h, hollow);
            if crowded(&grid) {
                return None;
            }
            seal(&mut grid);
            (!crowded(&grid)).then_some(grid)
        })
        .ok_or(Refused::Crowded)?;

    let targets = [
        (Pos { x: w / 4, y: h / 2 }, Dir::Right),
        (
            Pos {
                x: 3 * w / 4,
                y: h / 2,
            },
            Dir::Left,
        ),
        (Pos { x: w / 2, y: h / 4 }, Dir::Down),
        (
            Pos {
                x: w / 2,
                y: 3 * h / 4,
            },
            Dir::Up,
        ),
    ];
    let mut starts: Vec<(Pos, Dir)> = Vec::with_capacity(4);
    for (target, dir) in targets {
        let found = match nearest_start(&grid, target, dir, &starts, Some(START_SEARCH)) {
            Some(p) => Some(p),
            None => {
                carve(&mut grid, target, dir);
                seal(&mut grid);
                nearest_start(&grid, target, dir, &starts, None)
            }
        };
        // The lane always holds a run in every direction, so this is a guard only.
        starts.push((found.ok_or(Refused::Crowded)?, dir));
    }
    Ok(Fitted {
        grid,
        starts: [starts[0], starts[1], starts[2], starts[3]],
    })
}

/// The grid of `shapes` before sealing: border wall and every shape drawn, blocks as outlines
/// when `hollow`.
fn draw(shapes: &[Shape], w: u16, h: u16, hollow: bool) -> Grid {
    let mut grid = Grid::new(w, h);
    for x in 0..w {
        grid.set(Pos { x, y: 0 }, Tile::Wall);
        grid.set(Pos { x, y: h - 1 }, Tile::Wall);
    }
    for y in 0..h {
        grid.set(Pos { x: 0, y }, Tile::Wall);
        grid.set(Pos { x: w - 1, y }, Tile::Wall);
    }
    let m = 1 + MARGIN;
    let (iw, ih) = (w - 2 * m, h - 2 * m);
    for shape in shapes {
        crate::work::add(1);
        let Some(c) = cells(shape, iw, ih) else {
            continue;
        };
        let (x0, y0, x1, y1) = (c[0] + m, c[1] + m, c[2] + m, c[3] + m);
        let kind = match shape.kind {
            ShapeKind::Block if hollow => ShapeKind::Outline,
            kind => kind,
        };
        let mut wall = |x: u16, y: u16| grid.set(Pos { x, y }, Tile::Wall);
        match kind {
            ShapeKind::Line => {
                let [cx0, cy0, cx1, cy1] = shape_rect(shape);
                if (cx1 - cx0) * f32::from(iw) >= (cy1 - cy0) * f32::from(ih) {
                    let y = m + centre(cy0, cy1, ih);
                    for x in with_door(x0, x1, 3 * DOOR) {
                        wall(x, y);
                    }
                } else {
                    let x = m + centre(cx0, cx1, iw);
                    for y in with_door(y0, y1, 3 * DOOR) {
                        wall(x, y);
                    }
                }
            }
            ShapeKind::Outline => {
                for x in with_door(x0, x1, 2 * DOOR + 2) {
                    wall(x, y0);
                    wall(x, y1);
                }
                for y in with_door(y0, y1, 2 * DOOR + 2) {
                    wall(x0, y);
                    wall(x1, y);
                }
            }
            ShapeKind::Block | ShapeKind::Tower => {
                let tile = if kind == ShapeKind::Tower {
                    Tile::Tower
                } else {
                    Tile::Wall
                };
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        grid.set(Pos { x, y }, tile);
                    }
                }
            }
        }
    }
    grid
}

/// The rect of `shape` with sorted bounds clipped to `0..=1`; callers have checked it is
/// finite.
fn shape_rect(shape: &Shape) -> [f32; 4] {
    let [a, b, c, d] = shape.rect;
    [
        a.min(c).clamp(0.0, 1.0),
        b.min(d).clamp(0.0, 1.0),
        a.max(c).clamp(0.0, 1.0),
        b.max(d).clamp(0.0, 1.0),
    ]
}

/// The inclusive cell bounds `x0, y0, x1, y1` of `shape` in an inner area of `iw` by `ih`
/// cells, or `None` when the rect is not finite, does not overlap `0..=1` in x or in y, or
/// has no extent after clipping. A line may have zero height or width, not both.
fn cells(shape: &Shape, iw: u16, ih: u16) -> Option<[u16; 4]> {
    let [a, b, c, d] = shape.rect;
    if !shape.rect.iter().all(|v| v.is_finite()) {
        return None;
    }
    let overlaps = |p: f32, q: f32| p.max(q) >= 0.0 && p.min(q) <= 1.0;
    if !overlaps(a, c) || !overlaps(b, d) {
        return None;
    }
    let [x0, y0, x1, y1] = shape_rect(shape);
    let (dx, dy) = (x1 - x0, y1 - y0);
    let empty = match shape.kind {
        ShapeKind::Line => dx <= 0.0 && dy <= 0.0,
        _ => dx <= 0.0 || dy <= 0.0,
    };
    if empty {
        return None;
    }
    let (cx0, cx1) = span(x0, x1, iw);
    let (cy0, cy1) = span(y0, y1, ih);
    Some([cx0, cy0, cx1, cy1])
}

/// The inclusive cells `lo..=hi` that `a..=b` (in `0..=1`) covers out of `n`; at least one.
fn span(a: f32, b: f32, n: u16) -> (u16, u16) {
    let last = n - 1;
    let lo = ((a * f32::from(n)).floor() as u16).min(last);
    let hi = ((b * f32::from(n)).ceil() as u16)
        .saturating_sub(1)
        .min(last)
        .max(lo);
    (lo, hi)
}

/// The cell of the centre of `a..=b` (in `0..=1`) out of `n`.
fn centre(a: f32, b: f32, n: u16) -> u16 {
    (((a + b) / 2.0 * f32::from(n)).floor() as u16).min(n - 1)
}

/// The cells `lo..=hi`, without a door of [`DOOR`] cells in the middle when the run is longer
/// than `longer_than`.
fn with_door(lo: u16, hi: u16, longer_than: u16) -> impl Iterator<Item = u16> {
    let len = hi - lo + 1;
    let door = if len > longer_than {
        let start = lo + (len - DOOR) / 2;
        start..start + DOOR
    } else {
        0..0
    };
    (lo..=hi).filter(move |v| !door.contains(v))
}

/// True when walls and towers cover more than [`MAX_WALL_SHARE`] of the inner area.
fn crowded(grid: &Grid) -> bool {
    let m = 1 + MARGIN;
    let (w, h) = (grid.width(), grid.height());
    let mut taken = 0usize;
    for y in m..h - m {
        for x in m..w - m {
            if !grid.is_free(Pos { x, y }) {
                taken += 1;
            }
        }
    }
    let area = usize::from(w - 2 * m) * usize::from(h - 2 * m);
    taken as f32 > MAX_WALL_SHARE * area as f32
}

/// Makes every free cell that the lane does not reach a wall. The lane is the main area: no
/// shape maps into it, so it is always free and connected. A sealed area larger than the lane
/// still becomes wall, so the lane stays free.
fn seal(grid: &mut Grid) {
    let main = steer::region(grid, Pos { x: 1, y: 1 });
    let w = grid.width();
    for y in 0..grid.height() {
        for x in 0..w {
            let p = Pos { x, y };
            if grid.is_free(p) && !main[usize::from(y) * usize::from(w) + usize::from(x)] {
                grid.set(p, Tile::Wall);
            }
        }
    }
}

/// The first cell in breadth-first order from `target` (up to `limit` steps away, or anywhere
/// when `None`) that is a good start facing `dir`.
fn nearest_start(
    grid: &Grid,
    target: Pos,
    dir: Dir,
    starts: &[(Pos, Dir)],
    limit: Option<u16>,
) -> Option<Pos> {
    let w = usize::from(grid.width());
    let mut seen = vec![false; w * usize::from(grid.height())];
    let mut queue = VecDeque::from([(target, 0u16)]);
    seen[usize::from(target.y) * w + usize::from(target.x)] = true;
    while let Some((p, d)) = queue.pop_front() {
        if good_start(grid, p, dir, starts) {
            return Some(p);
        }
        if limit.is_some_and(|l| d >= l) {
            continue;
        }
        for step in Dir::ALL {
            if let Some(q) = grid.step(p, step) {
                let i = usize::from(q.y) * w + usize::from(q.x);
                if !seen[i] {
                    seen[i] = true;
                    queue.push_back((q, d + 1));
                }
            }
        }
    }
    None
}

/// The start cell `p` and the [`START_RUN`] cells ahead of it, or `None` when the run leaves
/// the grid.
fn run(grid: &Grid, p: Pos, dir: Dir) -> Option<Vec<Pos>> {
    let mut cells = vec![p];
    let mut at = p;
    for _ in 0..START_RUN {
        at = grid.step(at, dir)?;
        cells.push(at);
    }
    Some(cells)
}

/// True when `p` facing `dir` has a free run, is at least [`START_GAP`] from every earlier
/// start, and no start lies on another's run.
fn good_start(grid: &Grid, p: Pos, dir: Dir, starts: &[(Pos, Dir)]) -> bool {
    let Some(cells) = run(grid, p, dir) else {
        return false;
    };
    if !cells.iter().all(|c| grid.is_free(*c)) {
        return false;
    }
    starts.iter().all(|&(q, qd)| {
        p.x.abs_diff(q.x) + p.y.abs_diff(q.y) >= START_GAP
            && !cells.contains(&q)
            && run(grid, q, qd).is_none_or(|other| !other.contains(&p))
    })
}

/// Frees the run at `target` facing `dir` and a corridor behind it back to the lane, so the
/// run joins the main area.
fn carve(grid: &mut Grid, target: Pos, dir: Dir) {
    if let Some(cells) = run(grid, target, dir) {
        for c in cells {
            if on_inside(grid, c) {
                grid.set(c, Tile::Empty);
            }
        }
    }
    let back = dir.opposite();
    let mut at = target;
    while let Some(next) = grid.step(at, back) {
        if !on_inside(grid, next) || in_lane(grid, next) {
            break;
        }
        grid.set(next, Tile::Empty);
        at = next;
    }
}

/// True when `p` is not a border cell.
fn on_inside(grid: &Grid, p: Pos) -> bool {
    p.x > 0 && p.y > 0 && p.x < grid.width() - 1 && p.y < grid.height() - 1
}

/// True when `p` lies in the lane between the border and the inner area.
fn in_lane(grid: &Grid, p: Pos) -> bool {
    let m = 1 + MARGIN;
    p.x < m || p.y < m || p.x >= grid.width() - m || p.y >= grid.height() - m
}
