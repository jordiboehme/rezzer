// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The loop on the real terminal: draw, wait for a key or a resize until the next tick, tick.
//! Kept thin, because it cannot be tested without a terminal; the logic lives in [`App`].

use std::io;
use std::time::Instant;

use crossterm::event::{self, Event};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::app::{App, Flow, TICK, key_of};
use crate::plan::Rounds;
use crate::term::{Crossterm, Guard, install_restore_hook};

/// Runs `rounds` until the user quits. Every error is returned after the terminal is restored.
pub(crate) fn run(rounds: Rounds, seed: u64) -> Result<(), String> {
    // The restore hook must be the last panic hook, and nothing below catches panics.
    install_restore_hook();
    let mut guard = Guard::enter(Crossterm).map_err(terminal_error)?;
    let result = run_loop(rounds, seed);
    let left = guard.leave();
    result.and(left).map_err(terminal_error)
}

fn terminal_error(e: io::Error) -> String {
    format!("terminal error: {e}")
}

fn run_loop(rounds: Rounds, seed: u64) -> io::Result<()> {
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let size = terminal.size()?;
    let mut app = App::new(rounds, seed, size.width, size.height);
    let mut deadline = Instant::now() + TICK;
    loop {
        terminal.draw(|f| crate::ui::draw(&app, f.area(), f.buffer_mut()))?;
        if event::poll(deadline.saturating_duration_since(Instant::now()))? {
            match event::read()? {
                Event::Key(ev) => {
                    if let Some(key) = key_of(&ev)
                        && app.on_key(key) == Flow::Quit
                    {
                        return Ok(());
                    }
                }
                Event::Resize(cols, rows) => app.on_resize(cols, rows),
                _ => {}
            }
        }
        let now = Instant::now();
        if now >= deadline {
            app.on_tick();
            // No catch-up after a sleep: at most one extra tick, then the normal pace.
            deadline = (deadline + TICK).max(now);
        }
    }
}
