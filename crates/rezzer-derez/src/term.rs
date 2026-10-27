// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The terminal guard: raw mode and the alternate screen, always restored.
//!
//! [`Guard`] enters the screen and leaves it again on an explicit [`Guard::leave`], on drop
//! (a normal quit or an error that returns early) and on unwinding. [`install_restore_hook`]
//! also restores the terminal before the panic message is printed, so the message lands on the
//! normal screen and not in the alternate one, and it still works when a panic does not unwind.
//!
//! Ctrl-C needs no signal handler: in raw mode the terminal delivers it as a key, and the key
//! handling quits on it. A process that is killed (for example `kill -9`) runs none of this and
//! can leave the terminal in raw mode; that is accepted, and `reset` fixes it.
//!
//! The flag the hook checks is process-wide, so a panic on any thread restores the terminal,
//! also one that is caught (`catch_unwind`, a thread whose join result is ignored). derez must
//! therefore not catch panics while the guard is active, and this hook must stay the last
//! panic hook installed.

use std::io;
use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once};

use crossterm::cursor::{Hide, Show};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};

/// A screen the guard can enter and leave.
pub trait Screen {
    /// Switches the terminal to the state the grid needs.
    fn enter(&mut self) -> io::Result<()>;
    /// Restores the terminal to the state it had before [`Screen::enter`].
    fn leave(&mut self) -> io::Result<()>;
}

/// The real terminal: raw mode, the alternate screen and a hidden cursor.
pub struct Crossterm;

impl Screen for Crossterm {
    /// Raw mode first, then the alternate screen with the cursor hidden. When the second step
    /// fails, raw mode is switched off again before the error returns.
    fn enter(&mut self) -> io::Result<()> {
        enter_steps(
            terminal::enable_raw_mode,
            || execute!(io::stdout(), EnterAlternateScreen, Hide),
            || execute!(io::stdout(), Show, LeaveAlternateScreen),
            terminal::disable_raw_mode,
        )?;
        ACTIVE.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// The cursor and the normal screen first, then raw mode off. Both are tried even when the
    /// first fails; the first error is returned. Does nothing when the terminal is not entered
    /// (or the panic hook restored it already): leaving the alternate screen twice can move the
    /// cursor back above the panic message in xterm-like terminals.
    fn leave(&mut self) -> io::Result<()> {
        if !ACTIVE.swap(false, Ordering::SeqCst) {
            return Ok(());
        }
        leave_terminal()
    }
}

/// Leaves the real terminal, without looking at [`ACTIVE`].
fn leave_terminal() -> io::Result<()> {
    leave_steps(
        || execute!(io::stdout(), Show, LeaveAlternateScreen),
        terminal::disable_raw_mode,
    )
}

/// Set while the real terminal is entered, so the panic hook knows it must restore it.
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// What the panic hook calls to restore the terminal. Unit tests swap it for a recording one.
/// A `Mutex` because a plain swappable static would need `static mut`.
static RESTORE: Mutex<fn()> = Mutex::new(restore_terminal);

/// The hook has cleared [`ACTIVE`] already, so this leaves without the check in
/// [`Crossterm::leave`]; the guard's later drop then finds nothing to do.
fn restore_terminal() {
    let _ = leave_terminal();
}

/// Keeps a screen entered and leaves it when dropped.
pub struct Guard<S: Screen> {
    screen: S,
    active: bool,
}

impl<S: Screen> Guard<S> {
    /// Enters `screen`. On an error nothing is left entered and no guard is returned.
    pub fn enter(mut screen: S) -> io::Result<Guard<S>> {
        screen.enter()?;
        Ok(Guard {
            screen,
            active: true,
        })
    }

    /// Leaves the screen. Only the first call reaches the screen, also when it fails.
    pub fn leave(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        self.active = false;
        self.screen.leave()
    }
}

impl<S: Screen> Drop for Guard<S> {
    fn drop(&mut self) {
        let _ = self.leave();
    }
}

/// Wraps the current panic hook, once per process: while the real terminal is entered, a panic
/// first restores it and then calls the previous hook.
pub fn install_restore_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if ACTIVE.swap(false, Ordering::SeqCst) {
                let restore = *RESTORE.lock().unwrap_or_else(|e| e.into_inner());
                restore();
            }
            previous(info);
        }));
    });
}

/// Runs the steps of [`Crossterm::enter`]: `raw_on`, then `screen_on`. When `screen_on` fails,
/// `screen_off` (it may have been entered in part) and `raw_off` undo it before the error of
/// `screen_on` returns.
fn enter_steps(
    raw_on: impl FnOnce() -> io::Result<()>,
    screen_on: impl FnOnce() -> io::Result<()>,
    screen_off: impl FnOnce() -> io::Result<()>,
    raw_off: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    raw_on()?;
    if let Err(e) = screen_on() {
        let _ = screen_off();
        let _ = raw_off();
        return Err(e);
    }
    Ok(())
}

/// Runs the steps of [`Crossterm::leave`]: both, in order, and returns the first error.
fn leave_steps(
    screen_off: impl FnOnce() -> io::Result<()>,
    raw_off: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let screen = screen_off();
    let raw = raw_off();
    screen.and(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};
    use std::thread::{self, ThreadId};

    type Hook = Box<dyn Fn(&panic::PanicHookInfo<'_>) + Sync + Send>;

    static RECORD: StdMutex<Vec<&'static str>> = StdMutex::new(Vec::new());

    /// The thread of the hook test; only its panics are recorded.
    static TEST_THREAD: StdMutex<Option<ThreadId>> = StdMutex::new(None);

    fn on_test_thread() -> bool {
        *TEST_THREAD.lock().unwrap_or_else(|e| e.into_inner()) == Some(thread::current().id())
    }

    fn record_restore() {
        if on_test_thread() {
            RECORD
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push("restore");
        }
    }

    fn take_record() -> Vec<&'static str> {
        std::mem::take(&mut *RECORD.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// All hook checks live in this one test: the panic hook, `ACTIVE` and `RESTORE` are
    /// process-wide, so a second test would race with this one under `cargo test`. Panics from
    /// other threads are passed on to the original hook and not recorded. A panic on another
    /// thread while `ACTIVE` is set still clears it and can make this test fail; that is
    /// accepted because nextest runs each test in its own process, so no other test runs then.
    #[test]
    fn restore_hook_runs_before_the_previous_hook() {
        *TEST_THREAD.lock().unwrap_or_else(|e| e.into_inner()) = Some(thread::current().id());
        let original: Arc<Hook> = Arc::new(panic::take_hook());
        let forward = Arc::clone(&original);
        panic::set_hook(Box::new(move |info| {
            if on_test_thread() {
                RECORD
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push("previous");
            } else {
                forward(info);
            }
        }));
        *RESTORE.lock().unwrap_or_else(|e| e.into_inner()) = record_restore;
        take_record();

        install_restore_hook();
        install_restore_hook();

        ACTIVE.store(true, Ordering::SeqCst);
        let result = panic::catch_unwind(|| panic!("a deliberate panic with the screen entered"));
        assert!(result.is_err());
        let active_after = ACTIVE.load(Ordering::SeqCst);
        let with_screen = take_record();

        let result = panic::catch_unwind(|| panic!("a deliberate panic without the screen"));
        assert!(result.is_err());
        let without_screen = take_record();

        // Put the process back as it was before the asserts can fail: the real restore and a
        // hook chain that ends in the original hook. `ACTIVE` is clear, so ours does nothing.
        ACTIVE.store(false, Ordering::SeqCst);
        *RESTORE.lock().unwrap_or_else(|e| e.into_inner()) = restore_terminal;
        let _ = panic::take_hook();
        panic::set_hook(Box::new(move |info| original(info)));

        assert_eq!(with_screen, ["restore", "previous"]);
        assert!(!active_after);
        assert_eq!(without_screen, ["previous"]);
    }

    /// Steps of `enter` and `leave` with a recording log, for the order and error checks below.
    fn step(log: &StdMutex<Vec<&'static str>>, name: &'static str, fail: bool) -> io::Result<()> {
        log.lock().unwrap().push(name);
        if fail {
            Err(io::Error::other(name))
        } else {
            Ok(())
        }
    }

    #[test]
    fn enter_undoes_raw_mode_when_the_screen_fails() {
        let log = StdMutex::new(Vec::new());
        let err = enter_steps(
            || step(&log, "raw on", false),
            || step(&log, "screen on", true),
            || step(&log, "screen off", false),
            || step(&log, "raw off", false),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "screen on");
        assert_eq!(
            *log.lock().unwrap(),
            ["raw on", "screen on", "screen off", "raw off"]
        );
    }

    #[test]
    fn enter_stops_when_raw_mode_fails() {
        let log = StdMutex::new(Vec::new());
        let err = enter_steps(
            || step(&log, "raw on", true),
            || step(&log, "screen on", false),
            || step(&log, "screen off", false),
            || step(&log, "raw off", false),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "raw on");
        assert_eq!(*log.lock().unwrap(), ["raw on"]);
    }

    #[test]
    fn enter_runs_both_steps() {
        let log = StdMutex::new(Vec::new());
        enter_steps(
            || step(&log, "raw on", false),
            || step(&log, "screen on", false),
            || step(&log, "screen off", false),
            || step(&log, "raw off", false),
        )
        .unwrap();
        assert_eq!(*log.lock().unwrap(), ["raw on", "screen on"]);
    }

    #[test]
    fn leave_tries_both_steps_and_returns_the_first_error() {
        let log = StdMutex::new(Vec::new());
        let err = leave_steps(
            || step(&log, "screen off", true),
            || step(&log, "raw off", true),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "screen off");
        assert_eq!(*log.lock().unwrap(), ["screen off", "raw off"]);

        let log = StdMutex::new(Vec::new());
        let err = leave_steps(
            || step(&log, "screen off", false),
            || step(&log, "raw off", true),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "raw off");
        assert_eq!(*log.lock().unwrap(), ["screen off", "raw off"]);
    }
}
