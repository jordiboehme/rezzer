// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use rezzer_derez::grid::{Dir, Grid, Pos, Tile};
use rezzer_derez::rng::Rng;
use rezzer_derez::round::{MAX_QUEUED_TURNS, Round, Status};

fn pos(x: u16, y: u16) -> Pos {
    Pos { x, y }
}

/// A round with the user program alone on an empty grid.
fn alone(w: u16, h: u16, at: Pos, dir: Dir) -> Round {
    Round::new(Grid::new(w, h), &[(at, dir)], 0, 1)
}

#[test]
fn rng_is_deterministic() {
    let mut a = Rng::new(7);
    let mut b = Rng::new(7);
    let first: Vec<u64> = (0..100).map(|_| a.next_u64()).collect();
    let second: Vec<u64> = (0..100).map(|_| b.next_u64()).collect();
    assert_eq!(first, second);
    let mut c = Rng::new(8);
    let other: Vec<u64> = (0..100).map(|_| c.next_u64()).collect();
    assert_ne!(first, other);

    let mut r = Rng::new(42);
    let mut seen = [false; 10];
    for _ in 0..10_000 {
        let v = r.below(10);
        assert!(v < 10, "below(10) gave {v}");
        seen[v as usize] = true;
    }
    assert!(seen.iter().all(|s| *s), "every value below 10 shows up");

    let mut r = Rng::new(5);
    assert!((0..1000).all(|_| !r.chance(0)));
    assert!((0..1000).all(|_| r.chance(1000)));
}

#[test]
fn rng_matches_splitmix64() {
    // Reference values of SplitMix64 seeded with 0.
    let mut r = Rng::new(0);
    assert_eq!(r.next_u64(), 0xE220_A839_7B1D_CDAF);
    assert_eq!(r.next_u64(), 0x6E78_9E6A_A1B9_65F4);
}

#[test]
fn grid_basics() {
    let mut g = Grid::new(4, 3);
    assert_eq!((g.width(), g.height()), (4, 3));
    assert_eq!(g.get(pos(3, 2)), Tile::Empty);
    assert_eq!(
        g.get(pos(4, 0)),
        Tile::Wall,
        "off the grid counts as a wall"
    );
    assert_eq!(g.get(pos(0, 3)), Tile::Wall);
    g.set(pos(1, 1), Tile::Tower);
    assert_eq!(g.get(pos(1, 1)), Tile::Tower);
    assert!(!g.is_free(pos(1, 1)));
    assert!(g.is_free(pos(0, 0)));
    assert!(!g.is_free(pos(9, 9)));
    assert_eq!(g.step(pos(0, 0), Dir::Left), None);
    assert_eq!(g.step(pos(0, 0), Dir::Up), None);
    assert_eq!(g.step(pos(3, 2), Dir::Right), None);
    assert_eq!(g.step(pos(3, 2), Dir::Down), None);
    assert_eq!(g.step(pos(1, 1), Dir::Right), Some(pos(2, 1)));
    assert_eq!(g.step(pos(1, 1), Dir::Down), Some(pos(1, 2)));
    for d in Dir::ALL {
        assert_eq!(d.opposite().opposite(), d);
        assert_ne!(d.opposite(), d);
    }
}

#[test]
fn straight_run_leaves_a_trail() {
    let mut r = alone(10, 5, pos(1, 2), Dir::Right);
    r.steer(Dir::Right);
    assert_eq!(r.status(), Status::Running);
    for _ in 0..3 {
        assert!(r.tick().is_empty());
    }
    for x in 1..=4 {
        assert_eq!(r.grid().get(pos(x, 2)), Tile::Trail(0), "cell ({x},2)");
    }
    assert_eq!(r.grid().get(pos(5, 2)), Tile::Empty);
    assert_eq!(r.programs()[0].pos, pos(4, 2));
    assert_eq!(r.status(), Status::Running);
}

#[test]
fn wall_derezzes() {
    let mut g = Grid::new(10, 5);
    g.set(pos(3, 2), Tile::Wall);
    let mut r = Round::new(g, &[(pos(1, 2), Dir::Right)], 0, 1);
    r.steer(Dir::Right);
    assert!(r.tick().is_empty());
    assert_eq!(r.tick(), vec![0]);
    assert!(r.programs()[0].derezzed);
    assert_eq!(r.status(), Status::AllDerezzed);
    assert_eq!(r.grid().get(pos(3, 2)), Tile::Wall, "walls stay");

    // The edge of the grid derezzes too.
    let mut r = alone(3, 3, pos(2, 1), Dir::Right);
    r.steer(Dir::Right);
    assert_eq!(r.tick(), vec![0]);

    // So does a tower.
    let mut g = Grid::new(10, 5);
    g.set(pos(2, 2), Tile::Tower);
    let mut r = Round::new(g, &[(pos(1, 2), Dir::Right)], 0, 1);
    r.steer(Dir::Right);
    assert_eq!(r.tick(), vec![0]);
}

#[test]
fn own_trail_derezzes() {
    let mut r = alone(10, 10, pos(2, 2), Dir::Right);
    r.steer(Dir::Right);
    r.tick(); // (3,2)
    r.steer(Dir::Down);
    r.tick(); // (3,3)
    r.steer(Dir::Left);
    r.tick(); // (2,3)
    r.steer(Dir::Up);
    assert_eq!(r.tick(), vec![0], "(2,2) is its own trail");
    assert_eq!(r.status(), Status::AllDerezzed);
}

#[test]
fn same_cell_derezzes_both() {
    let mut r = Round::new(
        Grid::new(10, 5),
        &[(pos(2, 2), Dir::Right), (pos(4, 2), Dir::Left)],
        1,
        1,
    );
    r.steer(Dir::Right);
    let mut gone = r.tick();
    gone.sort_unstable();
    assert_eq!(gone, vec![0, 1]);
    assert!(r.programs().iter().all(|p| p.derezzed));
    assert_eq!(r.status(), Status::AllDerezzed);
    assert_eq!(r.grid().get(pos(3, 2)), Tile::Empty);
}

#[test]
fn swapping_cells_derezzes_both() {
    let mut r = Round::new(
        Grid::new(10, 5),
        &[(pos(2, 2), Dir::Right), (pos(3, 2), Dir::Left)],
        1,
        1,
    );
    r.steer(Dir::Right);
    let mut gone = r.tick();
    gone.sort_unstable();
    assert_eq!(gone, vec![0, 1]);
    assert_eq!(r.status(), Status::AllDerezzed);
}

#[test]
fn reverse_is_ignored() {
    let mut r = alone(10, 5, pos(1, 2), Dir::Right);
    r.steer(Dir::Left);
    assert_eq!(
        r.status(),
        Status::Running,
        "a dropped turn still starts the round"
    );
    assert!(r.tick().is_empty());
    assert_eq!(r.programs()[0].pos, pos(2, 2));
    assert_eq!(r.programs()[0].dir, Dir::Right);
}

#[test]
fn two_quick_turns_take_two_ticks() {
    let mut r = alone(10, 10, pos(2, 5), Dir::Right);
    r.steer(Dir::Up);
    r.steer(Dir::Left);
    assert!(r.tick().is_empty());
    assert_eq!(r.programs()[0].pos, pos(2, 4));
    assert!(r.tick().is_empty());
    assert_eq!(r.programs()[0].pos, pos(1, 4));
    assert!(!r.programs()[0].derezzed);
}

#[test]
fn queue_holds_two() {
    assert_eq!(MAX_QUEUED_TURNS, 2);
    let mut r = alone(10, 10, pos(5, 5), Dir::Right);
    r.steer(Dir::Up);
    r.steer(Dir::Left);
    r.steer(Dir::Down); // a third turn is dropped
    r.tick();
    assert_eq!(r.programs()[0].pos, pos(5, 4));
    r.tick();
    assert_eq!(r.programs()[0].pos, pos(4, 4));
    r.tick();
    assert_eq!(r.programs()[0].pos, pos(3, 4), "still moving left");
    assert_eq!(r.programs()[0].dir, Dir::Left);
}

#[test]
fn invalid_turn_is_not_queued() {
    let mut r = alone(10, 10, pos(5, 5), Dir::Right);
    r.steer(Dir::Left);
    r.steer(Dir::Up);
    r.tick();
    assert_eq!(r.programs()[0].pos, pos(5, 4));
    assert_eq!(r.programs()[0].dir, Dir::Up);
    // The same direction is not queued either.
    r.steer(Dir::Up);
    r.steer(Dir::Right);
    r.tick();
    assert_eq!(r.programs()[0].pos, pos(6, 4));
}

#[test]
fn derezzed_trail_is_cleared() {
    let mut g = Grid::new(12, 6);
    g.set(pos(8, 1), Tile::Wall);
    let mut r = Round::new(g, &[(pos(1, 4), Dir::Right), (pos(5, 1), Dir::Right)], 1, 1);
    r.steer(Dir::Right);
    assert!(r.tick().is_empty()); // program 1 at (6,1)
    assert!(r.tick().is_empty()); // program 1 at (7,1)
    assert_eq!(r.grid().get(pos(7, 1)), Tile::Trail(1));
    assert_eq!(r.tick(), vec![1]);
    for x in 0..12 {
        for y in 0..6 {
            assert_ne!(r.grid().get(pos(x, y)), Tile::Trail(1), "cell ({x},{y})");
        }
    }
    assert_eq!(r.grid().get(pos(8, 1)), Tile::Wall, "walls stay");
    assert_eq!(
        r.grid().get(pos(4, 4)),
        Tile::Trail(0),
        "the user trail stays"
    );
}

#[test]
fn cleared_and_derezzed_status() {
    // The other program runs into a wall: the user program is the last one left.
    let mut g = Grid::new(12, 6);
    g.set(pos(6, 1), Tile::Wall);
    let mut r = Round::new(g, &[(pos(1, 4), Dir::Right), (pos(5, 1), Dir::Right)], 1, 1);
    r.steer(Dir::Right);
    assert_eq!(r.tick(), vec![1]);
    assert_eq!(r.status(), Status::Cleared);
    let before = r.grid().clone();
    assert!(r.tick().is_empty(), "an ended round does not move");
    assert_eq!(r.grid(), &before);

    // The user program runs into a wall while another is left.
    let mut g = Grid::new(12, 6);
    g.set(pos(2, 4), Tile::Wall);
    let mut r = Round::new(g, &[(pos(1, 4), Dir::Right), (pos(5, 1), Dir::Right)], 1, 1);
    r.steer(Dir::Right);
    assert_eq!(r.tick(), vec![0]);
    assert_eq!(r.status(), Status::Derezzed);
    r.steer(Dir::Up);
    assert_eq!(
        r.status(),
        Status::Derezzed,
        "steering after the end does nothing"
    );
}

#[test]
fn ready_and_paused_do_not_move() {
    let mut r = alone(10, 5, pos(1, 2), Dir::Right);
    assert_eq!(r.status(), Status::Ready);
    assert_eq!(
        r.grid().get(pos(1, 2)),
        Tile::Trail(0),
        "the start cell is marked"
    );
    r.toggle_pause();
    assert_eq!(
        r.status(),
        Status::Ready,
        "pause does nothing before the start"
    );
    assert!(r.tick().is_empty());
    assert_eq!(r.programs()[0].pos, pos(1, 2));

    r.steer(Dir::Right);
    r.tick();
    assert_eq!(r.programs()[0].pos, pos(2, 2));
    r.toggle_pause();
    assert_eq!(r.status(), Status::Paused);
    r.steer(Dir::Down); // ignored while paused
    r.tick();
    r.tick();
    assert_eq!(r.programs()[0].pos, pos(2, 2));
    r.toggle_pause();
    assert_eq!(r.status(), Status::Running);
    r.tick();
    assert_eq!(r.programs()[0].pos, pos(3, 2));
}

#[test]
fn tick_with_asks_each_live_program_before_moving() {
    let mut r = Round::new(
        Grid::new(20, 10),
        &[
            (pos(1, 1), Dir::Right),
            (pos(10, 5), Dir::Right),
            (pos(10, 8), Dir::Right),
        ],
        2,
        9,
    );
    r.steer(Dir::Right);
    let mut asked = Vec::new();
    let gone = r.tick_with(|round, i| {
        // Nobody has moved yet when the choice is made.
        assert_eq!(round.programs()[0].pos, pos(1, 1));
        let _ = round.rng().next_u64();
        asked.push(i);
        Dir::Up
    });
    assert!(gone.is_empty());
    assert_eq!(asked, vec![1, 2]);
    assert_eq!(r.programs()[1].pos, pos(10, 4));
    assert_eq!(r.programs()[2].pos, pos(10, 7));
    assert_eq!(r.programs()[0].pos, pos(2, 1));
}

#[test]
fn same_seed_same_round() {
    let run = |seed: u64| {
        let mut r = Round::new(
            Grid::new(40, 20),
            &[
                (pos(2, 10), Dir::Right),
                (pos(37, 5), Dir::Left),
                (pos(37, 15), Dir::Left),
            ],
            2,
            seed,
        );
        let steers = [Dir::Up, Dir::Right, Dir::Down, Dir::Right, Dir::Up];
        for t in 0..50 {
            if t % 4 == 0 {
                r.steer(steers[(t / 4) % steers.len()]);
            }
            r.tick_with(|round, _| Dir::ALL[round.rng().below(4) as usize]);
        }
        (r.grid().clone(), r.status(), r.rng().next_u64())
    };
    assert_eq!(run(3), run(3));
}
