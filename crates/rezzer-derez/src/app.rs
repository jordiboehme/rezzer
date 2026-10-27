// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The state of a run: the current round, the tally of rounds and what the keys, ticks and
//! resizes do to them. Pure, so it is tested without a terminal.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;

use crate::grid::Dir;
use crate::plan::{MAX_GRID, MIN_GRID, Rounds, builtin, fit};
use crate::round::{Round, Status};
use crate::steer;

/// The time between two ticks.
pub const TICK: Duration = Duration::from_millis(70);
/// The smallest terminal: two columns per grid cell.
pub const MIN_COLS: u16 = 2 * MIN_GRID.0;
/// The smallest terminal: the grid plus the status line and the key line.
pub const MIN_ROWS: u16 = MIN_GRID.1 + 2;

const GREETING: &str = "Greetings, program. Steer to start.";
const PAUSED: &str = "Paused.  p: go on";
const CLEARED: &str = "End of Line.  r: next round  q: quit";
const DEREZZED: &str = "Derezzed.  r: next round  q: quit";
const ALL_DEREZZED: &str = "All derezzed.  r: next round  q: quit";
const CROWDED: &str = "This part is too crowded. Using the Grid.";

/// What a key does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Steer(Dir),
    Pause,
    Restart,
    Quit,
}

/// The key of a key event, if it has one. Only presses and repeats count: Windows also reports
/// releases. In raw mode Ctrl-C arrives as a key, and it quits. Other keys with Ctrl or Alt do
/// nothing; Shift and Caps Lock do not matter.
pub fn key_of(ev: &KeyEvent) -> Option<Key> {
    if !matches!(ev.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }
    let ch = match ev.code {
        KeyCode::Char(c) => Some(c.to_ascii_lowercase()),
        _ => None,
    };
    if ev.modifiers.contains(KeyModifiers::CONTROL) {
        return (ch == Some('c')).then_some(Key::Quit);
    }
    if ev
        .modifiers
        .intersects(KeyModifiers::ALT | KeyModifiers::SUPER | KeyModifiers::META)
    {
        return None;
    }
    match (ev.code, ch) {
        (KeyCode::Up, _) | (_, Some('w')) => Some(Key::Steer(Dir::Up)),
        (KeyCode::Down, _) | (_, Some('s')) => Some(Key::Steer(Dir::Down)),
        (KeyCode::Left, _) | (_, Some('a')) => Some(Key::Steer(Dir::Left)),
        (KeyCode::Right, _) | (_, Some('d')) => Some(Key::Steer(Dir::Right)),
        (_, Some('p')) => Some(Key::Pause),
        (_, Some('r' | ' ')) => Some(Key::Restart),
        (KeyCode::Esc, _) | (_, Some('q')) => Some(Key::Quit),
        _ => None,
    }
}

/// The rounds so far. `round` counts from 1; the others count ended rounds by how they ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tally {
    pub round: u32,
    pub cleared: u32,
    pub derezzed: u32,
    pub all_derezzed: u32,
}

/// Whether the run goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Go,
    Quit,
}

/// A run: the plans, the terminal size, the current round and the tally.
#[derive(Debug, Clone)]
pub struct App {
    rounds: Rounds,
    seed: u64,
    cols: u16,
    rows: u16,
    tally: Tally,
    /// `None` while the terminal is too small.
    round: Option<Round>,
    /// The current plan was refused as too crowded, so the round runs on the Grid.
    crowded: bool,
    /// The ids derezzed in the last tick.
    just_derezzed: Vec<u8>,
    /// The text for a terminal that is too small.
    too_small: String,
}

impl App {
    /// A run of `rounds` on a terminal of `cols` by `rows`, starting in round 1. Round `n` uses
    /// the seed `seed + n` (wrapping).
    pub fn new(rounds: Rounds, seed: u64, cols: u16, rows: u16) -> App {
        let mut app = App {
            rounds,
            seed,
            cols,
            rows,
            tally: Tally {
                round: 1,
                cleared: 0,
                derezzed: 0,
                all_derezzed: 0,
            },
            round: None,
            crowded: false,
            just_derezzed: Vec::new(),
            too_small: String::new(),
        };
        app.build();
        app
    }

    /// Acts on a key. Steering, pausing and restarting do nothing while the terminal is too
    /// small.
    pub fn on_key(&mut self, k: Key) -> Flow {
        match k {
            Key::Quit => return Flow::Quit,
            Key::Steer(d) => {
                if let Some(r) = &mut self.round {
                    r.steer(d);
                }
            }
            Key::Pause => {
                if let Some(r) = &mut self.round {
                    r.toggle_pause();
                }
            }
            Key::Restart => {
                if let Some(r) = &self.round {
                    if ended(r.status()) {
                        self.tally.round = self.tally.round.saturating_add(1);
                    }
                    self.build();
                }
            }
        }
        Flow::Go
    }

    /// One tick, in which the other programs steer by [`steer::choose`].
    pub fn on_tick(&mut self) {
        self.on_tick_with(steer::choose);
    }

    /// One tick, in which `choose` steers the other programs (see [`Round::tick_with`]). Does
    /// nothing unless a round is running. An end is counted in the tick that reaches it.
    #[doc(hidden)]
    pub fn on_tick_with(&mut self, choose: impl FnMut(&mut Round, usize) -> Dir) {
        let Some(r) = &mut self.round else {
            return;
        };
        if r.status() != Status::Running {
            return;
        }
        self.just_derezzed = r.tick_with(choose);
        let t = &mut self.tally;
        match r.status() {
            Status::Cleared => t.cleared = t.cleared.saturating_add(1),
            Status::Derezzed => t.derezzed = t.derezzed.saturating_add(1),
            Status::AllDerezzed => t.all_derezzed = t.all_derezzed.saturating_add(1),
            _ => {}
        }
    }

    /// Acts on a new terminal size. The same grid size keeps the round (only the grid moves to
    /// the new centre); another size builds the grid again and restarts the round, the tally
    /// kept.
    pub fn on_resize(&mut self, cols: u16, rows: u16) {
        let before = self.grid_size();
        (self.cols, self.rows) = (cols, rows);
        let after = self.grid_size();
        if self.round.is_none() || before != after {
            self.build();
        }
    }

    pub fn tally(&self) -> Tally {
        self.tally
    }

    /// The current round; `None` while the terminal is too small.
    pub fn round(&self) -> Option<&Round> {
        self.round.as_ref()
    }

    /// The text to show over the grid, if any.
    pub fn message(&self) -> Option<&str> {
        let Some(r) = &self.round else {
            return Some(&self.too_small);
        };
        match r.status() {
            Status::Ready => Some(GREETING),
            Status::Running => None,
            Status::Paused => Some(PAUSED),
            Status::Cleared => Some(CLEARED),
            Status::Derezzed => Some(DEREZZED),
            Status::AllDerezzed => Some(ALL_DEREZZED),
        }
    }

    /// Where the grid goes on the terminal, two columns per cell, centred between the status
    /// line and the key line; `None` while the terminal is too small.
    pub fn grid_area(&self) -> Option<Rect> {
        let (w, h) = self.grid_size()?;
        let width = 2 * w;
        Some(Rect::new(
            (self.cols - width) / 2,
            1 + (self.rows - 2 - h) / 2,
            width,
            h,
        ))
    }

    /// What the current plan was made from.
    pub fn label(&self) -> &str {
        self.rounds
            .plans
            .get(self.plan_index())
            .map_or("the Grid", |p| p.label.as_str())
    }

    /// The source file name, as the run got it.
    pub fn name(&self) -> &str {
        &self.rounds.name
    }

    /// A note for the status line: the current plan is too crowded, or (in the first round only)
    /// the file gives no grid.
    pub fn note(&self) -> Option<&str> {
        if self.crowded {
            Some(CROWDED)
        } else if self.tally.round == 1 {
            self.rounds.note.as_deref()
        } else {
            None
        }
    }

    /// A note for the second line of the message while the round waits for the first key: the
    /// current plan is too crowded, or (in the first round only) the file gives no grid, without
    /// the file name, which the status line shows.
    pub fn ready_note(&self) -> Option<&str> {
        if self.round.as_ref()?.status() != Status::Ready {
            return None;
        }
        if self.crowded {
            Some(CROWDED)
        } else if self.tally.round == 1 {
            self.rounds.short_note.as_deref()
        } else {
            None
        }
    }

    /// The ids derezzed in the last tick.
    pub fn just_derezzed(&self) -> &[u8] {
        &self.just_derezzed
    }

    /// The grid size in cells for the terminal; `None` below [`MIN_COLS`] by [`MIN_ROWS`].
    fn grid_size(&self) -> Option<(u16, u16)> {
        (self.cols >= MIN_COLS && self.rows >= MIN_ROWS).then(|| {
            (
                (self.cols / 2).min(MAX_GRID.0),
                (self.rows - 2).min(MAX_GRID.1),
            )
        })
    }

    fn plan_index(&self) -> usize {
        // `Rounds` always holds at least one plan; the guard keeps an empty one from panicking.
        let n = self.rounds.plans.len().max(1);
        (self.tally.round.saturating_sub(1) as usize) % n
    }

    /// Builds the round of the current round number in `Ready`, or none while the terminal is
    /// too small.
    fn build(&mut self) {
        self.just_derezzed.clear();
        self.crowded = false;
        let Some((w, h)) = self.grid_size() else {
            self.round = None;
            self.too_small = format!(
                "Make the terminal at least {MIN_COLS}x{MIN_ROWS}. It is {}x{}.",
                self.cols, self.rows
            );
            return;
        };
        let fitted = match self
            .rounds
            .plans
            .get(self.plan_index())
            .map(|p| fit(p, w, h))
        {
            Some(Ok(f)) => f,
            _ => {
                self.crowded = !self.rounds.plans.is_empty();
                fit(&builtin(), w, h).expect("the Grid fits every size from MIN_GRID up")
            }
        };
        let others = 1 + self.tally.cleared.min(2) as usize;
        let seed = self.seed.wrapping_add(u64::from(self.tally.round));
        self.round = Some(Round::new(fitted.grid, &fitted.starts, others, seed));
    }
}

fn ended(s: Status) -> bool {
    matches!(s, Status::Cleared | Status::Derezzed | Status::AllDerezzed)
}
