// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The grid of cells that programs move on.

/// A cell position; `x` grows to the right, `y` downward.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pos {
    pub x: u16,
    pub y: u16,
}

/// A direction of movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    /// Every direction.
    pub const ALL: [Dir; 4] = [Dir::Up, Dir::Down, Dir::Left, Dir::Right];

    /// The reverse direction.
    pub fn opposite(self) -> Dir {
        match self {
            Dir::Up => Dir::Down,
            Dir::Down => Dir::Up,
            Dir::Left => Dir::Right,
            Dir::Right => Dir::Left,
        }
    }
}

/// What a cell holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tile {
    Empty,
    Wall,
    Tower,
    /// The trail of the program with this id.
    Trail(u8),
}

/// A rectangle of cells. Everything off the grid counts as [`Tile::Wall`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    w: u16,
    h: u16,
    cells: Vec<Tile>,
}

impl Grid {
    /// A grid of `w` by `h` empty cells.
    pub fn new(w: u16, h: u16) -> Grid {
        Grid {
            w,
            h,
            cells: vec![Tile::Empty; usize::from(w) * usize::from(h)],
        }
    }

    pub fn width(&self) -> u16 {
        self.w
    }

    pub fn height(&self) -> u16 {
        self.h
    }

    fn index(&self, p: Pos) -> Option<usize> {
        (p.x < self.w && p.y < self.h)
            .then(|| usize::from(p.y) * usize::from(self.w) + usize::from(p.x))
    }

    /// The tile at `p`; [`Tile::Wall`] off the grid.
    pub fn get(&self, p: Pos) -> Tile {
        self.index(p).map_or(Tile::Wall, |i| self.cells[i])
    }

    /// Sets the tile at `p`; does nothing off the grid.
    pub fn set(&mut self, p: Pos, t: Tile) {
        if let Some(i) = self.index(p) {
            self.cells[i] = t;
        }
    }

    /// The neighbour of `p` in direction `d`, or `None` when it is off the grid.
    pub fn step(&self, p: Pos, d: Dir) -> Option<Pos> {
        let next = match d {
            Dir::Up => Pos {
                x: p.x,
                y: p.y.checked_sub(1)?,
            },
            Dir::Down => Pos {
                x: p.x,
                y: p.y.checked_add(1)?,
            },
            Dir::Left => Pos {
                x: p.x.checked_sub(1)?,
                y: p.y,
            },
            Dir::Right => Pos {
                x: p.x.checked_add(1)?,
                y: p.y,
            },
        };
        self.index(next).map(|_| next)
    }

    /// True when `p` is on the grid and empty.
    pub fn is_free(&self, p: Pos) -> bool {
        self.get(p) == Tile::Empty
    }

    /// Replaces every cell for which `f` is true with [`Tile::Empty`].
    pub(crate) fn clear_where(&mut self, f: impl Fn(Tile) -> bool) {
        for t in &mut self.cells {
            if f(*t) {
                *t = Tile::Empty;
            }
        }
    }
}
