// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::VecDeque;

use rezzer_derez::grid::{Dir, Grid, Pos, Tile};
use rezzer_derez::rng::Rng;
use rezzer_derez::round::{Round, Status};
use rezzer_derez::steer::{self, CHASE_RANGE, FILL_CAP, SLIP_PER_MILLE};

fn pos(x: u16, y: u16) -> Pos {
    Pos { x, y }
}

/// A grid from text rows: `#` is a wall, anything else is free.
fn grid_from(rows: &[&str]) -> Grid {
    let mut g = Grid::new(rows[0].len() as u16, rows.len() as u16);
    for (y, row) in rows.iter().enumerate() {
        for (x, c) in row.chars().enumerate() {
            if c == '#' {
                g.set(pos(x as u16, y as u16), Tile::Wall);
            }
        }
    }
    g
}

/// The free neighbours of `p` that are not behind `dir`.
fn safe_dirs(g: &Grid, p: Pos, dir: Dir) -> Vec<Dir> {
    Dir::ALL
        .into_iter()
        .filter(|d| *d != dir.opposite() && g.step(p, *d).is_some_and(|n| g.is_free(n)))
        .collect()
}

#[test]
fn constants_are_as_planned() {
    assert_eq!(SLIP_PER_MILLE, 80);
    assert_eq!(CHASE_RANGE, 8);
    assert_eq!(FILL_CAP, 4000);
}

#[test]
fn never_turns_into_a_wall_when_free() {
    let mut checked = 0;
    for seed in 0..2000u64 {
        let mut r = Rng::new(seed ^ 0xD5);
        let mut g = Grid::new(20, 12);
        for y in 0..12 {
            for x in 0..20 {
                if r.chance(150) {
                    g.set(pos(x, y), Tile::Wall);
                }
            }
        }
        let mut free_cell = |g: &Grid| loop {
            let p = pos(r.below(20) as u16, r.below(12) as u16);
            if g.is_free(p) {
                break p;
            }
        };
        let user = free_cell(&g);
        let other = loop {
            let p = free_cell(&g);
            if p != user {
                break p;
            }
        };
        let dir = Dir::ALL[r.below(4) as usize];
        let mut round = Round::new(g, &[(user, Dir::Right), (other, dir)], 1, seed);
        let safe = safe_dirs(round.grid(), other, dir);
        let d = steer::choose(&mut round, 1);
        if safe.is_empty() {
            assert_eq!(d, dir, "seed {seed}: trapped keeps its direction");
        } else {
            checked += 1;
            assert!(safe.contains(&d), "seed {seed}: chose {d:?}, safe {safe:?}");
        }
    }
    assert!(checked > 1500, "only {checked} seeds had a safe direction");
}

/// The user program sits far away in a closed cell, so no chase starts.
fn dead_end_room() -> Grid {
    grid_from(&[
        "..........####################",
        "..........####################",
        "..........####################",
        "..........####################",
        "..........####################",
        "..........####################",
        "####.#########################",
        "#....#########################",
        "############################.#",
    ])
}

#[test]
fn avoids_a_dead_end() {
    // The program at (4,7) heads up: left is a 3-cell pocket, straight on a 60-cell room,
    // right a wall.
    let mut straight = 0;
    for seed in 0..1000u64 {
        let mut round = Round::new(
            dead_end_room(),
            &[(pos(28, 8), Dir::Up), (pos(4, 7), Dir::Up)],
            1,
            seed,
        );
        let d = steer::choose(&mut round, 1);
        assert!(matches!(d, Dir::Up | Dir::Left), "seed {seed}: {d:?}");
        if d == Dir::Up {
            straight += 1;
        }
    }
    assert!(straight >= 850, "straight in {straight} of 1000");

    // Heading left at the same cell: straight on is now the pocket, a turn up the room.
    let mut up = 0;
    for seed in 0..1000u64 {
        let mut round = Round::new(
            dead_end_room(),
            &[(pos(28, 8), Dir::Up), (pos(4, 7), Dir::Left)],
            1,
            seed,
        );
        if steer::choose(&mut round, 1) == Dir::Up {
            up += 1;
        }
    }
    assert!(up >= 850, "up in {up} of 1000");
}

#[test]
fn looks_two_steps_ahead() {
    // Heading right at (2,2): up enters (2,1), whose only other way out is closed, so that cell
    // ends the program one step later. Down (2,3) leads on. Right is a wall.
    let g = grid_from(&[
        "##############",
        "##.###########",
        "##.###########",
        "##...........#",
        "##...........#",
        "##############",
    ]);
    for seed in 0..2000u64 {
        let mut round = Round::new(
            g.clone(),
            &[(pos(12, 4), Dir::Left), (pos(2, 2), Dir::Right)],
            1,
            seed,
        );
        assert_eq!(
            steer::choose(&mut round, 1),
            Dir::Down,
            "seed {seed}: never into a cell without a way out"
        );
    }
}

/// The share of seeds in which the program at (10,11), heading up on an open grid, turns right.
fn right_turns(user: (Pos, Dir)) -> u32 {
    let mut right = 0;
    for seed in 0..1000u64 {
        let mut round = Round::new(Grid::new(30, 20), &[user, (pos(10, 11), Dir::Up)], 1, seed);
        if steer::choose(&mut round, 1) == Dir::Right {
            right += 1;
        }
    }
    right
}

#[test]
fn chases_a_near_user_program() {
    // The cell in front of the user program is (15,11): only a right turn gets closer to it.
    let near = right_turns((pos(15, 12), Dir::Up));
    assert!(near >= 850, "right in {near} of 1000 when near");
    // Out of range the program keeps its direction on equal space.
    let far = right_turns((pos(28, 1), Dir::Up));
    assert!(far <= 100, "right in {far} of 1000 when far");
}

/// A full round with the planned steering for the other programs and a fixed user steer list.
fn run_round(seed: u64) -> (Grid, Vec<(Pos, Dir, bool)>, Status) {
    let mut r = Round::new(
        Grid::new(40, 20),
        &[
            (pos(2, 10), Dir::Right),
            (pos(37, 3), Dir::Left),
            (pos(37, 17), Dir::Left),
        ],
        2,
        seed,
    );
    let steers = [Dir::Up, Dir::Right, Dir::Down, Dir::Right];
    for t in 0..800 {
        r.steer(steers[(t / 5) % steers.len()]);
        r.tick();
        if r.status() != Status::Running {
            break;
        }
    }
    let programs = r
        .programs()
        .iter()
        .map(|p| (p.pos, p.dir, p.derezzed))
        .collect();
    (r.grid().clone(), programs, r.status())
}

#[test]
fn deterministic_per_seed() {
    let outcomes: Vec<_> = (0..20).map(run_round).collect();
    for (seed, first) in outcomes.iter().enumerate() {
        assert_eq!(first, &run_round(seed as u64), "seed {seed}");
    }
    let distinct = outcomes
        .iter()
        .enumerate()
        .filter(|(i, o)| !outcomes[..*i].contains(o))
        .count();
    assert!(distinct >= 2, "20 seeds gave {distinct} outcome(s)");
}

#[test]
fn keeps_direction_when_trapped() {
    let g = grid_from(&["#####", "#..##", "#####", "....."]);
    let mut round = Round::new(g, &[(pos(0, 3), Dir::Right), (pos(2, 1), Dir::Right)], 1, 5);
    assert_eq!(steer::choose(&mut round, 1), Dir::Right);
    round.steer(Dir::Right);
    assert_eq!(round.tick(), vec![1]);
    assert_eq!(round.status(), Status::Cleared);
}

/// Free cells reachable from `from` over 4 neighbours, `blocked` counted as a wall. Cells in
/// `seen` are marked and not counted again.
fn fill(g: &Grid, from: Pos, blocked: Pos, seen: &mut [bool]) -> usize {
    let at = |p: Pos| usize::from(p.y) * usize::from(g.width()) + usize::from(p.x);
    if from == blocked || !g.is_free(from) || seen[at(from)] {
        return 0;
    }
    seen[at(from)] = true;
    let mut queue = VecDeque::from([from]);
    let mut n = 0;
    while let Some(p) = queue.pop_front() {
        n += 1;
        for d in Dir::ALL {
            if let Some(q) = g.step(p, d)
                && q != blocked
                && g.is_free(q)
                && !seen[at(q)]
            {
                seen[at(q)] = true;
                queue.push_back(q);
            }
        }
    }
    n
}

/// The space rule without slips or chase, for the user program: free space two steps ahead,
/// halved where another head can enter. Ties keep the direction, then the order of `Dir::ALL`.
fn user_choice(round: &Round) -> Dir {
    let g = round.grid();
    let me = &round.programs()[0];
    let mut seen = vec![false; usize::from(g.width()) * usize::from(g.height())];
    let mut rate = |c: Pos| {
        // A second step already reached from another one lies in the same area.
        seen.fill(false);
        let space = Dir::ALL
            .into_iter()
            .filter_map(|d| g.step(c, d))
            .map(|c2| fill(g, c2, c, &mut seen))
            .max()
            .unwrap_or(0);
        let contested = round.programs()[1..]
            .iter()
            .any(|p| !p.derezzed && Dir::ALL.into_iter().any(|d| g.step(p.pos, d) == Some(c)));
        if contested { space / 2 } else { space }
    };
    let mut best: Option<(usize, Dir)> = None;
    for d in std::iter::once(me.dir).chain(Dir::ALL) {
        if d == me.dir.opposite() {
            continue;
        }
        let Some(c) = g.step(me.pos, d).filter(|c| g.is_free(*c)) else {
            continue;
        };
        let s = rate(c);
        if best.is_none_or(|(b, _)| s > b) {
            best = Some((s, d));
        }
    }
    best.map_or(me.dir, |(_, d)| d)
}

/// 60 by 30 with a wall border and four 3x3 wall blocks at the quarter points.
fn quarter_blocks() -> Grid {
    let mut g = Grid::new(60, 30);
    for x in 0..60 {
        g.set(pos(x, 0), Tile::Wall);
        g.set(pos(x, 29), Tile::Wall);
    }
    for y in 0..30 {
        g.set(pos(0, y), Tile::Wall);
        g.set(pos(59, y), Tile::Wall);
    }
    for (cx, cy) in [(15, 7), (44, 7), (15, 22), (44, 22)] {
        for y in cy - 1..=cy + 1 {
            for x in cx - 1..=cx + 1 {
                g.set(pos(x, y), Tile::Wall);
            }
        }
    }
    g
}

#[test]
fn can_be_beaten_but_not_trivially() {
    let grid = quarter_blocks();
    let free = (0..30)
        .flat_map(|y| (0..60).map(move |x| pos(x, y)))
        .filter(|p| grid.is_free(*p))
        .count();
    let (mut derezzed, mut cleared, mut all) = (0, 0, 0);
    for seed in 0..200u64 {
        let mut r = Round::new(
            grid.clone(),
            &[(pos(10, 15), Dir::Right), (pos(49, 15), Dir::Left)],
            1,
            seed,
        );
        let mut ticks = 0;
        loop {
            let d = user_choice(&r);
            r.steer(d);
            r.tick();
            ticks += 1;
            assert!(
                ticks <= free,
                "seed {seed}: the round ran past {free} ticks"
            );
            match r.status() {
                Status::Running => {}
                Status::Derezzed => {
                    derezzed += 1;
                    break;
                }
                Status::Cleared => {
                    cleared += 1;
                    break;
                }
                Status::AllDerezzed => {
                    all += 1;
                    break;
                }
                other => panic!("seed {seed}: status {other:?}"),
            }
        }
    }
    let summary = format!("derezzed {derezzed}, cleared {cleared}, all derezzed {all} of 200");
    eprintln!("{summary}");
    assert!((40..=160).contains(&derezzed), "{summary}");
}
