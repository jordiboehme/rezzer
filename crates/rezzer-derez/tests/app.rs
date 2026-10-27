// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use rezzer_derez::app::{App, Flow, Key, MIN_COLS, MIN_ROWS, TICK, Tally, key_of};
use rezzer_derez::grid::Dir;
use rezzer_derez::plan::{Plan, Refused, Rounds, Shape, ShapeKind, builtin, fit};
use rezzer_derez::round::{Round, Status};

fn grid_rounds() -> Rounds {
    Rounds {
        name: String::new(),
        plans: vec![builtin()],
        note: None,
        short_note: None,
    }
}

/// The Grid under another label, so a test can tell the plans apart.
fn labelled(label: &str) -> Plan {
    Plan {
        label: label.to_string(),
        ..builtin()
    }
}

fn status(app: &App) -> Status {
    app.round().expect("a round").status()
}

/// Ends the round with the user program as the last one left: the other programs turn back
/// and run into their own start cells by the second tick.
fn clear_round(app: &mut App) {
    app.on_key(Key::Steer(Dir::Right));
    for _ in 0..2 {
        app.on_tick_with(|r, i| r.programs()[i].dir.opposite());
    }
    assert_eq!(status(app), Status::Cleared);
}

/// Ends the round with the user program derezzed: it runs a small square back into its own
/// start cell while the other programs go straight on.
fn derez_round(app: &mut App) {
    for d in [Dir::Up, Dir::Left, Dir::Down, Dir::Right] {
        app.on_key(Key::Steer(d));
        app.on_tick_with(|r, i| r.programs()[i].dir);
    }
    assert_eq!(status(app), Status::Derezzed);
}

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Press)
}

#[test]
fn constants_are_as_planned() {
    assert_eq!(TICK.as_millis(), 70);
    assert_eq!((MIN_COLS, MIN_ROWS), (60, 20));
}

#[test]
fn first_steer_starts() {
    let mut app = App::new(grid_rounds(), 1, 80, 30);
    assert_eq!(status(&app), Status::Ready);
    assert_eq!(app.message(), Some("Greetings, program. Steer to start."));
    assert_eq!(
        app.tally(),
        Tally {
            round: 1,
            cleared: 0,
            derezzed: 0,
            all_derezzed: 0
        }
    );
    let before = app.round().unwrap().programs().to_vec();
    app.on_tick();
    assert_eq!(app.round().unwrap().programs(), &before[..]);

    assert_eq!(app.on_key(Key::Steer(Dir::Up)), Flow::Go);
    assert_eq!(status(&app), Status::Running);
    assert_eq!(app.message(), None);
    app.on_tick();
    assert_ne!(app.round().unwrap().programs()[0].pos, before[0].pos);
}

#[test]
fn pause_stops_ticks() {
    let mut app = App::new(grid_rounds(), 2, 80, 30);
    app.on_key(Key::Steer(Dir::Right));
    app.on_tick();
    assert_eq!(app.on_key(Key::Pause), Flow::Go);
    assert_eq!(status(&app), Status::Paused);
    assert_eq!(app.message(), Some("Paused.  p: go on"));
    let before = app.round().unwrap().programs().to_vec();
    for _ in 0..5 {
        app.on_tick();
    }
    assert_eq!(app.round().unwrap().programs(), &before[..]);
    app.on_key(Key::Pause);
    assert_eq!(status(&app), Status::Running);
    app.on_tick();
    assert_ne!(app.round().unwrap().programs(), &before[..]);
}

#[test]
fn restart_mid_round_keeps_the_tally() {
    let mut app = App::new(grid_rounds(), 3, 80, 30);
    let fresh = app.round().unwrap().clone();
    app.on_key(Key::Steer(Dir::Right));
    for _ in 0..3 {
        app.on_tick();
    }
    let tally = app.tally();
    assert_eq!(app.on_key(Key::Restart), Flow::Go);
    assert_eq!(app.tally(), tally);
    let again = app.round().unwrap();
    assert_eq!(again.status(), Status::Ready);
    assert_eq!(again.grid(), fresh.grid());
    assert_eq!(again.programs(), fresh.programs());

    // A paused round restarts the same way.
    app.on_key(Key::Steer(Dir::Right));
    app.on_tick();
    app.on_key(Key::Pause);
    app.on_key(Key::Restart);
    assert_eq!(app.tally(), tally);
    assert_eq!(app.round().unwrap().programs(), fresh.programs());
}

#[test]
fn next_round_cycles_plans() {
    let rounds = Rounds {
        name: "notes.md".to_string(),
        plans: vec![labelled("part 1"), labelled("part 2"), labelled("part 3")],
        note: None,
        short_note: None,
    };
    let mut app = App::new(rounds, 4, 80, 30);
    let mut seen = Vec::new();
    for n in 1..=4 {
        assert_eq!(app.tally().round, n);
        seen.push(app.label().to_string());
        derez_round(&mut app);
        assert_eq!(app.on_key(Key::Restart), Flow::Go);
        assert_eq!(status(&app), Status::Ready);
    }
    assert_eq!(seen, ["part 1", "part 2", "part 3", "part 1"]);
    assert_eq!(app.tally().round, 5);
    assert_eq!(app.tally().derezzed, 4);
}

#[test]
fn round_seed_follows_the_round() {
    // The same seed and round give the same round; the next round gets the next seed.
    let mut a = App::new(grid_rounds(), 9, 80, 30);
    let b = App::new(grid_rounds(), 10, 80, 30);
    derez_round(&mut a);
    a.on_key(Key::Restart);
    let f = fit(&builtin(), 40, 28).unwrap();
    let mut expected = Round::new(f.grid, &f.starts, 1, 11);
    let mut got = a.round().unwrap().clone();
    let mut other = b.round().unwrap().clone();
    for r in [&mut expected, &mut got, &mut other] {
        r.steer(Dir::Up);
        for _ in 0..30 {
            r.tick();
        }
    }
    // Seed 9 in round 2 and seed 10 in round 1 both give round seed 11.
    assert_eq!(got.programs(), expected.programs());
    assert_eq!(other.programs(), expected.programs());
    assert_eq!(got.grid(), expected.grid());
}

#[test]
fn cleared_round_adds_a_program() {
    let mut app = App::new(grid_rounds(), 5, 80, 30);
    let mut counts = Vec::new();
    for _ in 0..5 {
        let others = app.round().unwrap().programs().len() - 1;
        let cleared = app.tally().cleared;
        assert_eq!(others as u32, 1 + cleared.min(2), "after {cleared} cleared");
        counts.push(others);
        clear_round(&mut app);
        app.on_key(Key::Restart);
    }
    assert_eq!(counts, [1, 2, 3, 3, 3]);
    assert_eq!(app.tally().cleared, 5);

    // A derezzed round adds none.
    let mut app = App::new(grid_rounds(), 5, 80, 30);
    derez_round(&mut app);
    app.on_key(Key::Restart);
    assert_eq!(app.round().unwrap().programs().len(), 2);
}

#[test]
fn ends_count_once() {
    let mut app = App::new(grid_rounds(), 6, 80, 30);
    clear_round(&mut app);
    for _ in 0..5 {
        app.on_tick();
    }
    assert_eq!(app.tally().cleared, 1);
    assert_eq!(app.message(), Some("End of Line.  r: next round  q: quit"));

    let mut app = App::new(grid_rounds(), 6, 80, 30);
    derez_round(&mut app);
    app.on_tick();
    assert_eq!(app.tally().derezzed, 1);
    assert_eq!(app.message(), Some("Derezzed.  r: next round  q: quit"));

    // The user program and the other programs all run into trails in the same tick.
    let mut app = App::new(grid_rounds(), 6, 80, 30);
    let mut tick = 0;
    for d in [Dir::Up, Dir::Left, Dir::Down, Dir::Right] {
        app.on_key(Key::Steer(d));
        tick += 1;
        let back = tick == 4;
        app.on_tick_with(move |r, i| {
            let dir = r.programs()[i].dir;
            if back { dir.opposite() } else { dir }
        });
    }
    assert_eq!(status(&app), Status::AllDerezzed);
    assert_eq!(app.tally().all_derezzed, 1);
    assert_eq!(app.tally().derezzed, 0);
    assert_eq!(app.message(), Some("All derezzed.  r: next round  q: quit"));
}

#[test]
fn release_events_do_nothing() {
    for code in [
        KeyCode::Up,
        KeyCode::Char('q'),
        KeyCode::Char('p'),
        KeyCode::Esc,
    ] {
        let ev = KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Release);
        assert_eq!(key_of(&ev), None, "{code:?}");
    }
    let repeat = KeyEvent::new_with_kind(KeyCode::Left, KeyModifiers::NONE, KeyEventKind::Repeat);
    assert_eq!(key_of(&repeat), Some(Key::Steer(Dir::Left)));
    assert_eq!(key_of(&press(KeyCode::Left)), Some(Key::Steer(Dir::Left)));
}

#[test]
fn ctrl_c_quits() {
    for c in ['c', 'C'] {
        let ev =
            KeyEvent::new_with_kind(KeyCode::Char(c), KeyModifiers::CONTROL, KeyEventKind::Press);
        assert_eq!(key_of(&ev), Some(Key::Quit));
    }
    assert_eq!(key_of(&press(KeyCode::Char('c'))), None);
    // Other Ctrl and Alt keys do nothing.
    let ctrl_w = KeyEvent::new_with_kind(
        KeyCode::Char('w'),
        KeyModifiers::CONTROL,
        KeyEventKind::Press,
    );
    assert_eq!(key_of(&ctrl_w), None);
    let alt_w = KeyEvent::new_with_kind(KeyCode::Char('w'), KeyModifiers::ALT, KeyEventKind::Press);
    assert_eq!(key_of(&alt_w), None);

    let mut app = App::new(grid_rounds(), 7, 80, 30);
    assert_eq!(app.on_key(Key::Quit), Flow::Quit);
    assert_eq!(key_of(&press(KeyCode::Esc)), Some(Key::Quit));
    assert_eq!(key_of(&press(KeyCode::Char('q'))), Some(Key::Quit));
    assert_eq!(key_of(&press(KeyCode::Char('p'))), Some(Key::Pause));
    assert_eq!(key_of(&press(KeyCode::Char('r'))), Some(Key::Restart));
    assert_eq!(key_of(&press(KeyCode::Char(' '))), Some(Key::Restart));
}

#[test]
fn wasd_and_arrows_steer() {
    let cases = [
        (KeyCode::Up, Dir::Up),
        (KeyCode::Down, Dir::Down),
        (KeyCode::Left, Dir::Left),
        (KeyCode::Right, Dir::Right),
        (KeyCode::Char('w'), Dir::Up),
        (KeyCode::Char('s'), Dir::Down),
        (KeyCode::Char('a'), Dir::Left),
        (KeyCode::Char('d'), Dir::Right),
        (KeyCode::Char('W'), Dir::Up),
        (KeyCode::Char('S'), Dir::Down),
        (KeyCode::Char('A'), Dir::Left),
        (KeyCode::Char('D'), Dir::Right),
    ];
    for (code, dir) in cases {
        assert_eq!(key_of(&press(code)), Some(Key::Steer(dir)), "{code:?}");
    }
    // Shift with an upper case letter steers too.
    let shift_w =
        KeyEvent::new_with_kind(KeyCode::Char('W'), KeyModifiers::SHIFT, KeyEventKind::Press);
    assert_eq!(key_of(&shift_w), Some(Key::Steer(Dir::Up)));
}

#[test]
fn resize_below_minimum_waits_and_back_restarts() {
    let mut app = App::new(grid_rounds(), 8, 80, 30);
    app.on_key(Key::Steer(Dir::Right));
    for _ in 0..3 {
        app.on_tick();
    }
    let tally = app.tally();

    app.on_resize(59, 20);
    assert!(app.round().is_none());
    assert_eq!(app.grid_area(), None);
    assert_eq!(
        app.message(),
        Some("Make the terminal at least 60x20. It is 59x20.")
    );
    app.on_tick();
    assert_eq!(app.on_key(Key::Steer(Dir::Up)), Flow::Go);
    assert_eq!(app.on_key(Key::Restart), Flow::Go);
    assert!(app.round().is_none());
    assert_eq!(app.tally(), tally);

    app.on_resize(60, 19);
    assert_eq!(
        app.message(),
        Some("Make the terminal at least 60x20. It is 60x19.")
    );
    assert_eq!(app.on_key(Key::Quit), Flow::Quit);

    app.on_resize(80, 30);
    assert_eq!(status(&app), Status::Ready);
    assert_eq!(app.tally(), tally);
    assert_eq!(app.message(), Some("Greetings, program. Steer to start."));
    let g = app.round().unwrap().grid();
    assert_eq!((g.width(), g.height()), (40, 28));

    // Too small from the start, and a terminal of no size at all.
    let app = App::new(grid_rounds(), 8, 0, 0);
    assert!(app.round().is_none());
    assert_eq!(
        app.message(),
        Some("Make the terminal at least 60x20. It is 0x0.")
    );
}

#[test]
fn resize_to_another_grid_size_restarts_the_round() {
    let mut app = App::new(grid_rounds(), 8, 80, 30);
    app.on_key(Key::Steer(Dir::Right));
    app.on_tick();
    let tally = app.tally();
    app.on_resize(100, 30);
    assert_eq!(status(&app), Status::Ready);
    assert_eq!(app.tally(), tally);
    let g = app.round().unwrap().grid();
    assert_eq!((g.width(), g.height()), (50, 28));
}

#[test]
fn same_size_resize_keeps_the_round() {
    let mut app = App::new(grid_rounds(), 9, 300, 80);
    app.on_key(Key::Steer(Dir::Right));
    app.on_tick();
    let before = app.round().unwrap().clone();
    assert_eq!(app.grid_area(), Some(Rect::new(30, 10, 240, 60)));
    app.on_resize(302, 80);
    let after = app.round().unwrap();
    assert_eq!(after.status(), Status::Running);
    assert_eq!(after.programs(), before.programs());
    assert_eq!(after.grid(), before.grid());
    assert_eq!(app.grid_area(), Some(Rect::new(31, 10, 240, 60)));
}

#[test]
fn large_terminal_centres_the_grid() {
    let app = App::new(grid_rounds(), 10, 400, 100);
    let g = app.round().unwrap().grid();
    assert_eq!((g.width(), g.height()), (120, 60));
    assert_eq!(app.grid_area(), Some(Rect::new(80, 20, 240, 60)));

    // At the smallest size the grid fills the rows between the two text rows.
    let app = App::new(grid_rounds(), 10, 60, 20);
    assert_eq!(app.grid_area(), Some(Rect::new(0, 1, 60, 18)));
    // An odd column is left over on the right.
    let app = App::new(grid_rounds(), 10, 61, 20);
    assert_eq!(app.grid_area(), Some(Rect::new(0, 1, 60, 18)));
}

#[test]
fn crowded_plan_uses_the_grid_with_a_note() {
    let crowded = Plan {
        label: "page 2".to_string(),
        shapes: vec![Shape {
            kind: ShapeKind::Tower,
            rect: [0.0, 0.0, 1.0, 1.0],
        }],
    };
    assert_eq!(fit(&crowded, 40, 28), Err(Refused::Crowded));
    let rounds = Rounds {
        name: "drawing.pdf".to_string(),
        plans: vec![labelled("page 1"), crowded],
        note: None,
        short_note: None,
    };
    let mut app = App::new(rounds, 11, 80, 30);
    assert_eq!(app.note(), None);
    assert_eq!(app.ready_note(), None);
    derez_round(&mut app);
    app.on_key(Key::Restart);
    assert_eq!(app.label(), "page 2");
    assert_eq!(
        app.note(),
        Some("This part is too crowded. Using the Grid.")
    );
    assert_eq!(
        app.ready_note(),
        Some("This part is too crowded. Using the Grid.")
    );
    let f = fit(&builtin(), 40, 28).unwrap();
    let expected = Round::new(f.grid, &f.starts, 1, 13);
    assert_eq!(app.round().unwrap().grid(), expected.grid());
    assert_eq!(app.round().unwrap().programs(), expected.programs());
}

#[test]
fn file_note_shows_in_the_first_round_only() {
    let rounds = Rounds {
        name: "x.bin".to_string(),
        plans: vec![builtin()],
        note: Some("x.bin gives no grid: no reader. Using the Grid.".to_string()),
        short_note: Some("No grid from this file: no reader.".to_string()),
    };
    let mut app = App::new(rounds, 12, 80, 30);
    assert_eq!(
        app.note(),
        Some("x.bin gives no grid: no reader. Using the Grid.")
    );
    assert_eq!(app.ready_note(), Some("No grid from this file: no reader."));
    derez_round(&mut app);
    assert!(app.note().is_some());
    // Only while the round waits for the first key.
    assert_eq!(app.ready_note(), None);
    app.on_key(Key::Restart);
    assert_eq!(app.note(), None);
    assert_eq!(app.ready_note(), None);
}
