// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! How the other programs steer: by the free space they can still reach two steps ahead, with
//! a chase of a near user program and a few random slips, so they can be beaten but not
//! trivially. A slip takes a random safe direction that still has a way out (any safe one when
//! none has).

use std::collections::VecDeque;

use crate::grid::{Dir, Grid, Pos};
use crate::round::Round;

/// The chance in 1000 that a program takes a random safe direction that still has a way out
/// (any safe one when none has) instead of the best one.
pub const SLIP_PER_MILLE: u32 = 80;

/// The Manhattan distance up to which a program chases the user program.
pub const CHASE_RANGE: u16 = 8;

/// The most cells one flood fill counts.
pub const FILL_CAP: usize = 4000;

/// The direction of the program at index `id` (from 1) for the next tick. Never reverses.
/// Without a free cell ahead or to the side it keeps its direction (and is derezzed). Else it
/// slips now and then (see [`SLIP_PER_MILLE`]); otherwise it takes the most free space two
/// steps ahead, or chases a near user program when that costs little space.
pub fn choose(round: &mut Round, id: usize) -> Dir {
    let grid = round.grid();
    let me = &round.programs()[id];
    let (at, dir) = (me.pos, me.dir);

    // Candidates in a fixed order: straight on first, then the two turns.
    let safe: Vec<(Dir, Pos)> = [dir, turn_left(dir), turn_left(dir).opposite()]
        .into_iter()
        .filter_map(|d| {
            grid.step(at, d)
                .filter(|c| grid.is_free(*c))
                .map(|c| (d, c))
        })
        .collect();
    if safe.is_empty() {
        return dir;
    }

    if round.rng().chance(SLIP_PER_MILLE) {
        // A slip never enters a cell without a way out while another safe cell has one: that
        // would be a sure end, not a slip.
        let grid = round.grid();
        let open: Vec<Dir> = safe
            .iter()
            .filter(|(_, c)| has_exit(grid, *c))
            .map(|(d, _)| *d)
            .collect();
        let pool: Vec<Dir> = if open.is_empty() {
            safe.iter().map(|(d, _)| *d).collect()
        } else {
            open
        };
        return pool[round.rng().below(pool.len() as u32) as usize];
    }

    let grid = round.grid();
    let mut fill = Fill::new(grid);
    // (direction, cell, space)
    let rated: Vec<(Dir, Pos, usize)> = safe
        .iter()
        .map(|&(d, c)| {
            let space = fill.space_after(grid, c);
            // A cell another head can enter this tick may end both programs. Any cell next to
            // a head counts: the one behind it is its own trail after the first tick, so this is
            // broader only on that first tick, on purpose: it keeps the check simple.
            let contested = round
                .programs()
                .iter()
                .enumerate()
                .any(|(j, p)| j != id && !p.derezzed && distance(p.pos, c) == 1);
            (d, c, if contested { space / 2 } else { space })
        })
        .collect();
    let best = rated.iter().map(|r| r.2).max().unwrap_or(0);

    let mut winners: Vec<Dir> = Vec::new();
    let user = &round.programs()[0];
    if !user.derezzed && distance(at, user.pos) <= CHASE_RANGE {
        let target = grid.step(user.pos, user.dir).unwrap_or(user.pos);
        let now = distance(at, target);
        // Enough space (at least 80 % of the best) and closer to the cell in front of the
        // user program; of those the closest, then the most space.
        let chasing: Vec<&(Dir, Pos, usize)> = rated
            .iter()
            .filter(|r| r.2 * 5 >= best * 4 && distance(r.1, target) < now)
            .collect();
        if let Some(nearest) = chasing.iter().map(|r| distance(r.1, target)).min() {
            let nearest: Vec<_> = chasing
                .into_iter()
                .filter(|r| distance(r.1, target) == nearest)
                .collect();
            // Tie-break among the chasing candidates: the closest, then the most space.
            let most = nearest.iter().map(|r| r.2).max().unwrap_or(0);
            winners = nearest
                .iter()
                .filter(|r| r.2 == most)
                .map(|r| r.0)
                .collect();
        }
    }
    if winners.is_empty() {
        winners = rated.iter().filter(|r| r.2 == best).map(|r| r.0).collect();
    }

    if winners.contains(&dir) {
        dir
    } else {
        winners[round.rng().below(winners.len() as u32) as usize]
    }
}

/// The number of free cells reachable from `from` over the four neighbours, at most `cap`.
/// Zero when `from` is not free. Each call fills with its own buffers; the steering itself
/// uses `Fill` to reuse them.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn reachable(grid: &Grid, from: Pos, cap: usize) -> usize {
    let mut fill = Fill::new(grid);
    fill.count(grid, from, None, cap)
}

/// Flood fill buffers, reused for every fill of one choice.
struct Fill {
    seen: Vec<bool>,
    queue: VecDeque<Pos>,
}

impl Fill {
    fn new(grid: &Grid) -> Fill {
        Fill {
            seen: vec![false; usize::from(grid.width()) * usize::from(grid.height())],
            queue: VecDeque::new(),
        }
    }

    fn index(grid: &Grid, p: Pos) -> usize {
        usize::from(p.y) * usize::from(grid.width()) + usize::from(p.x)
    }

    /// The space after moving to `c`: the most free cells reachable from a free neighbour of
    /// `c`, with `c` itself taken; 0 when `c` has no free neighbour.
    fn space_after(&mut self, grid: &Grid, c: Pos) -> usize {
        self.seen.fill(false);
        let mut best = 0;
        for d in Dir::ALL {
            let Some(c2) = grid.step(c, d) else { continue };
            // A neighbour an earlier fill reached lies in the same area: same count.
            if grid.is_free(c2) && !self.seen[Fill::index(grid, c2)] {
                best = best.max(self.count(grid, c2, Some(c), FILL_CAP));
            }
        }
        best
    }

    /// Counts from `from`, with `blocked` taken as a wall. Cells marked by an earlier fill
    /// since the last clear are not counted again.
    fn count(&mut self, grid: &Grid, from: Pos, blocked: Option<Pos>, cap: usize) -> usize {
        if Some(from) == blocked || !grid.is_free(from) || cap == 0 {
            return 0;
        }
        self.queue.clear();
        self.seen[Fill::index(grid, from)] = true;
        self.queue.push_back(from);
        let mut n = 0;
        while let Some(p) = self.queue.pop_front() {
            n += 1;
            if n >= cap {
                break;
            }
            for d in Dir::ALL {
                if let Some(q) = grid.step(p, d)
                    && Some(q) != blocked
                    && grid.is_free(q)
                {
                    let i = Fill::index(grid, q);
                    if !self.seen[i] {
                        self.seen[i] = true;
                        self.queue.push_back(q);
                    }
                }
            }
        }
        n
    }
}

/// True when `c` has a free neighbour.
fn has_exit(grid: &Grid, c: Pos) -> bool {
    Dir::ALL
        .into_iter()
        .any(|d| grid.step(c, d).is_some_and(|n| grid.is_free(n)))
}

fn distance(a: Pos, b: Pos) -> u16 {
    a.x.abs_diff(b.x) + a.y.abs_diff(b.y)
}

/// The direction a quarter turn to the left of `d` (y grows downward).
fn turn_left(d: Dir) -> Dir {
    match d {
        Dir::Up => Dir::Left,
        Dir::Left => Dir::Down,
        Dir::Down => Dir::Right,
        Dir::Right => Dir::Up,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Tile;

    #[test]
    fn reachable_counts_free_cells_up_to_the_cap() {
        let mut g = Grid::new(6, 4);
        // A wall column at x = 2 splits off a 2 by 4 area on the left.
        for y in 0..4 {
            g.set(Pos { x: 2, y }, Tile::Wall);
        }
        assert_eq!(reachable(&g, Pos { x: 0, y: 0 }, FILL_CAP), 8);
        assert_eq!(reachable(&g, Pos { x: 5, y: 3 }, FILL_CAP), 12);
        assert_eq!(reachable(&g, Pos { x: 5, y: 3 }, 5), 5);
        assert_eq!(reachable(&g, Pos { x: 2, y: 1 }, FILL_CAP), 0, "a wall");
        assert_eq!(
            reachable(&g, Pos { x: 9, y: 9 }, FILL_CAP),
            0,
            "off the grid"
        );
        assert_eq!(reachable(&g, Pos { x: 0, y: 0 }, 0), 0);
    }

    #[test]
    fn space_after_takes_the_cell_and_the_larger_side() {
        // Entering (2,1) on a 5 by 3 grid with a wall column at x = 2 except that cell splits
        // the grid into two areas of 6 cells each.
        let mut g = Grid::new(5, 3);
        g.set(Pos { x: 2, y: 0 }, Tile::Wall);
        g.set(Pos { x: 2, y: 2 }, Tile::Wall);
        g.set(Pos { x: 4, y: 2 }, Tile::Wall);
        let mut fill = Fill::new(&g);
        assert_eq!(fill.space_after(&g, Pos { x: 2, y: 1 }), 6);
        // A cell with no free neighbour has no space.
        let mut g = Grid::new(3, 1);
        g.set(Pos { x: 0, y: 0 }, Tile::Wall);
        g.set(Pos { x: 2, y: 0 }, Tile::Wall);
        assert_eq!(Fill::new(&g).space_after(&g, Pos { x: 1, y: 0 }), 0);
    }
}
