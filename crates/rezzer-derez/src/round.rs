// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One round: programs move one cell per tick and leave a trail; a program that runs into a
//! wall, a tower, a trail or the edge of the grid is derezzed.

use std::collections::VecDeque;

use crate::grid::{Dir, Grid, Pos, Tile};
use crate::rng::Rng;

/// The most turns of the user program that wait for the next ticks.
pub const MAX_QUEUED_TURNS: usize = 2;

/// A program on the grid. Id 0 is the user program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub id: u8,
    pub pos: Pos,
    pub dir: Dir,
    pub derezzed: bool,
}

/// Where a round stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Waiting for the first steer.
    Ready,
    Running,
    Paused,
    /// The user program is the last one left.
    Cleared,
    /// The user program is derezzed and another program is left.
    Derezzed,
    /// No program is left.
    AllDerezzed,
}

/// A round on one grid.
#[derive(Debug, Clone)]
pub struct Round {
    grid: Grid,
    programs: Vec<Program>,
    turns: VecDeque<Dir>,
    status: Status,
    rng: Rng,
}

impl Round {
    /// A round with the user program and `programs` other programs; program `i` starts at
    /// `starts[i]`, and its start cell is marked as its trail.
    ///
    /// # Panics
    ///
    /// When `starts` has fewer than `programs + 1` entries, or `programs` is more than 255.
    pub fn new(mut grid: Grid, starts: &[(Pos, Dir)], programs: usize, seed: u64) -> Round {
        let count = programs + 1;
        assert!(count <= 256, "at most 255 other programs");
        assert!(
            starts.len() >= count,
            "{count} programs need {count} start positions, got {}",
            starts.len()
        );
        let programs = starts[..count]
            .iter()
            .enumerate()
            .map(|(i, &(pos, dir))| {
                let id = i as u8;
                grid.set(pos, Tile::Trail(id));
                Program {
                    id,
                    pos,
                    dir,
                    derezzed: false,
                }
            })
            .collect();
        Round {
            grid,
            programs,
            turns: VecDeque::with_capacity(MAX_QUEUED_TURNS),
            status: Status::Ready,
            rng: Rng::new(seed),
        }
    }

    /// Turns the user program. The first steer starts the round, even when the turn itself is
    /// dropped. A turn into the same direction or its reverse (against the last queued turn, or
    /// the direction moved last) is dropped, as is a turn beyond [`MAX_QUEUED_TURNS`]. While
    /// paused or after the end, steering does nothing.
    pub fn steer(&mut self, d: Dir) {
        match self.status {
            Status::Ready => self.status = Status::Running,
            Status::Running => {}
            _ => return,
        }
        if self.turns.len() >= MAX_QUEUED_TURNS {
            return;
        }
        let last = self.turns.back().copied().unwrap_or(self.programs[0].dir);
        if d != last && d != last.opposite() {
            self.turns.push_back(d);
        }
    }

    /// Pauses a running round or goes on with a paused one; does nothing otherwise.
    pub fn toggle_pause(&mut self) {
        self.status = match self.status {
            Status::Running => Status::Paused,
            Status::Paused => Status::Running,
            other => other,
        };
    }

    /// One tick in which the other programs keep their direction. Returns the ids derezzed in
    /// this tick.
    pub fn tick(&mut self) -> Vec<u8> {
        self.tick_with(|round, i| round.programs[i].dir)
    }

    /// One tick. `choose` gives the direction of each other program that is not derezzed (by
    /// index, from 1), before anyone moves. Returns the ids derezzed in this tick. Does nothing
    /// unless the round is running.
    pub fn tick_with(&mut self, mut choose: impl FnMut(&mut Round, usize) -> Dir) -> Vec<u8> {
        if self.status != Status::Running {
            return Vec::new();
        }
        let mut chosen = Vec::with_capacity(self.programs.len());
        for i in 1..self.programs.len() {
            if !self.programs[i].derezzed {
                chosen.push((i, choose(self, i)));
            }
        }
        for (i, d) in chosen {
            self.programs[i].dir = d;
        }
        // A running round has a live user program.
        if let Some(d) = self.turns.pop_front() {
            self.programs[0].dir = d;
        }

        let live: Vec<usize> = (0..self.programs.len())
            .filter(|&i| !self.programs[i].derezzed)
            .collect();
        let next: Vec<Option<Pos>> = live
            .iter()
            .map(|&i| {
                let p = &self.programs[i];
                self.grid.step(p.pos, p.dir)
            })
            .collect();
        let mut gone = vec![false; live.len()];
        for (a, &na) in next.iter().enumerate() {
            match na {
                None => gone[a] = true,
                Some(cell) if !self.grid.is_free(cell) => gone[a] = true,
                Some(_) => {}
            }
            for b in a + 1..live.len() {
                let (pa, pb) = (self.programs[live[a]].pos, self.programs[live[b]].pos);
                let same = na.is_some() && na == next[b];
                // A head is always marked as a trail, so the not-empty rule above already
                // derezzes a swap; this is a second safeguard should that ever change.
                let swap = na == Some(pb) && next[b] == Some(pa);
                if same || swap {
                    gone[a] = true;
                    gone[b] = true;
                }
            }
        }

        let mut derezzed = Vec::new();
        for (k, &i) in live.iter().enumerate() {
            let p = &mut self.programs[i];
            match (gone[k], next[k]) {
                (false, Some(cell)) => {
                    p.pos = cell;
                    self.grid.set(cell, Tile::Trail(p.id));
                }
                _ => {
                    p.derezzed = true;
                    derezzed.push(p.id);
                }
            }
        }
        if !derezzed.is_empty() {
            self.grid
                .clear_where(|t| matches!(t, Tile::Trail(id) if derezzed.contains(&id)));
        }
        self.status = self.after_tick();
        derezzed
    }

    fn after_tick(&self) -> Status {
        let others = &self.programs[1..];
        let others_left = others.iter().any(|p| !p.derezzed);
        match (self.programs[0].derezzed, others_left) {
            (true, true) => Status::Derezzed,
            (true, false) => Status::AllDerezzed,
            // With no other program from the start, the round runs until the user program is
            // derezzed.
            (false, false) if !others.is_empty() => Status::Cleared,
            (false, _) => Status::Running,
        }
    }

    pub fn status(&self) -> Status {
        self.status
    }

    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    pub fn programs(&self) -> &[Program] {
        &self.programs
    }

    /// The round's seeded generator, for the steering of the other programs.
    pub fn rng(&mut self) -> &mut Rng {
        &mut self.rng
    }
}
