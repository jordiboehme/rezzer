// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::VecDeque;
use std::time::Instant;

use rezzer_derez::grid::{Dir, Grid, Pos, Tile};
use rezzer_derez::plan::{
    Fitted, MARGIN, MAX_GRID, MAX_SHAPES, MAX_WALL_SHARE, MIN_GRID, Plan, Refused, START_RUN,
    Shape, ShapeKind, builtin, fit,
};
use rezzer_derez::rng::Rng;

fn pos(x: u16, y: u16) -> Pos {
    Pos { x, y }
}

fn plan(shapes: Vec<Shape>) -> Plan {
    Plan {
        label: "invented".to_string(),
        shapes,
    }
}

fn shape(kind: ShapeKind, rect: [f32; 4]) -> Shape {
    Shape { kind, rect }
}

/// The free cells reachable from `from`, as a mask indexed by `y * w + x`. Written here on
/// its own, so the test does not share the code it checks.
fn flood(g: &Grid, from: Pos) -> Vec<bool> {
    let w = usize::from(g.width());
    let mut seen = vec![false; w * usize::from(g.height())];
    if !g.is_free(from) {
        return seen;
    }
    let mut queue = VecDeque::from([from]);
    seen[usize::from(from.y) * w + usize::from(from.x)] = true;
    while let Some(p) = queue.pop_front() {
        let near = [
            (p.x.wrapping_sub(1), p.y),
            (p.x + 1, p.y),
            (p.x, p.y.wrapping_sub(1)),
            (p.x, p.y + 1),
        ];
        for (x, y) in near {
            let q = pos(x, y);
            if x < g.width() && y < g.height() && g.is_free(q) {
                let i = usize::from(y) * w + usize::from(x);
                if !seen[i] {
                    seen[i] = true;
                    queue.push_back(q);
                }
            }
        }
    }
    seen
}

fn ahead(p: Pos, d: Dir, n: u16) -> Pos {
    match d {
        Dir::Up => pos(p.x, p.y - n),
        Dir::Down => pos(p.x, p.y + n),
        Dir::Left => pos(p.x - n, p.y),
        Dir::Right => pos(p.x + n, p.y),
    }
}

fn border_is_wall(g: &Grid) -> bool {
    let (w, h) = (g.width(), g.height());
    (0..w).all(|x| g.get(pos(x, 0)) == Tile::Wall && g.get(pos(x, h - 1)) == Tile::Wall)
        && (0..h).all(|y| g.get(pos(0, y)) == Tile::Wall && g.get(pos(w - 1, y)) == Tile::Wall)
}

/// Every lane cell between the border and the inner area.
fn lane(g: &Grid) -> Vec<Pos> {
    let (w, h) = (g.width(), g.height());
    let inner = |v: u16, n: u16| v > MARGIN && v < n - 1 - MARGIN;
    let mut cells = Vec::new();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            if !(inner(x, w) && inner(y, h)) {
                cells.push(pos(x, y));
            }
        }
    }
    cells
}

/// Each start and the `START_RUN` cells ahead of it are free, no start lies on another's run
/// and the starts are at least 4 cells apart (Manhattan).
fn assert_starts(f: &Fitted) {
    for (i, &(p, d)) in f.starts.iter().enumerate() {
        for n in 0..=START_RUN {
            let c = ahead(p, d, n);
            assert!(
                f.grid.is_free(c),
                "start {i} at {p:?} {d:?}: {c:?} not free"
            );
            for (j, &(q, _)) in f.starts.iter().enumerate() {
                assert!(i == j || q != c, "start {j} lies on the run of start {i}");
            }
        }
        for &(q, _) in &f.starts[..i] {
            assert!(
                p.x.abs_diff(q.x) + p.y.abs_diff(q.y) >= 4,
                "{p:?} near {q:?}"
            );
        }
    }
}

/// The user program's start reaches every free cell and all four starts.
fn assert_one_area(f: &Fitted) {
    let g = &f.grid;
    let seen = flood(g, f.starts[0].0);
    let w = usize::from(g.width());
    for y in 0..g.height() {
        for x in 0..g.width() {
            let i = usize::from(y) * w + usize::from(x);
            assert!(!g.is_free(pos(x, y)) || seen[i], "({x},{y}) is sealed off");
        }
    }
    for &(p, _) in &f.starts {
        assert!(seen[usize::from(p.y) * w + usize::from(p.x)]);
    }
}

/// Walls and towers, sealed pockets included, cover at most `MAX_WALL_SHARE` of the inner
/// area.
fn assert_share(g: &Grid) {
    let m = 1 + MARGIN;
    let area = usize::from(g.width() - 2 * m) * usize::from(g.height() - 2 * m);
    let taken = walls_in_inner_area(g);
    assert!(
        taken as f32 <= MAX_WALL_SHARE * area as f32,
        "{taken} of {area} cells taken"
    );
}

fn walls_in_inner_area(g: &Grid) -> usize {
    let (w, h) = (g.width(), g.height());
    let m = 1 + MARGIN;
    let mut n = 0;
    for y in m..h - m {
        for x in m..w - m {
            if !g.is_free(pos(x, y)) {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn builtin_fits_every_size() {
    let p = builtin();
    assert_eq!(p.label, "the Grid");
    for w in (MIN_GRID.0..=MAX_GRID.0).step_by(3) {
        for h in (MIN_GRID.1..=MAX_GRID.1).step_by(3) {
            let f = fit(&p, w, h).unwrap_or_else(|e| panic!("{w}x{h}: {e:?}"));
            assert_eq!((f.grid.width(), f.grid.height()), (w, h));
            assert!(border_is_wall(&f.grid), "{w}x{h}");
            // The centre lies inside the box, which must be open, not sealed.
            let m = 1 + MARGIN;
            let centre = pos(m + (w - 2 * m) / 2, m + (h - 2 * m) / 2);
            assert!(f.grid.is_free(centre), "{w}x{h}: centre {centre:?}");
            assert_starts(&f);
            assert_one_area(&f);
        }
    }
    // A larger size is clamped.
    let f = fit(&p, 500, 300).unwrap();
    assert_eq!((f.grid.width(), f.grid.height()), MAX_GRID);
}

#[test]
fn margin_lane_is_free() {
    let p = plan(vec![shape(ShapeKind::Block, [0.0, 0.0, 1.0, 1.0])]);
    for (w, h) in [(30, 18), (80, 40), (120, 60)] {
        match fit(&p, w, h) {
            Ok(f) => {
                assert!(border_is_wall(&f.grid));
                for c in lane(&f.grid) {
                    assert!(f.grid.is_free(c), "{w}x{h}: lane cell {c:?}");
                }
                assert_one_area(&f);
            }
            Err(e) => assert_eq!(e, Refused::Crowded, "{w}x{h}"),
        }
    }
}

#[test]
fn starts_are_free_with_room() {
    let crowded_middle = plan(vec![
        shape(ShapeKind::Block, [0.15, 0.4, 0.35, 0.6]),
        shape(ShapeKind::Block, [0.65, 0.4, 0.85, 0.6]),
        shape(ShapeKind::Tower, [0.45, 0.15, 0.55, 0.35]),
        shape(ShapeKind::Tower, [0.45, 0.65, 0.55, 0.85]),
    ]);
    for p in [builtin(), plan(Vec::new()), crowded_middle] {
        for (w, h) in [(30, 18), (57, 31), (80, 40), (120, 60)] {
            let f = fit(&p, w, h).unwrap();
            assert_starts(&f);
            assert_eq!(f.starts[0].1, Dir::Right, "the user program faces right");
        }
    }
}

/// A random rect in unit coordinates; now and then reversed, flat or partly off the square.
fn random_rect(r: &mut Rng) -> [f32; 4] {
    let mut u = || r.below(1201) as f32 / 1000.0 - 0.1;
    let (x0, y0) = (u(), u());
    let (dx, dy) = (u() * 0.4, u() * 0.4);
    [x0, y0, x0 + dx, y0 + dy]
}

#[test]
fn no_sealed_start_area() {
    let kinds = [
        ShapeKind::Line,
        ShapeKind::Outline,
        ShapeKind::Block,
        ShapeKind::Tower,
    ];
    let (mut ok, mut crowded) = (0, 0);
    for i in 0..500 {
        let mut r = Rng::new(i);
        let n = r.below(41);
        let shapes = (0..n)
            .map(|_| {
                let kind = kinds[r.below(4) as usize];
                shape(kind, random_rect(&mut r))
            })
            .collect();
        let p = plan(shapes);
        for (w, h) in [(30, 18), (80, 40), (120, 60)] {
            match fit(&p, w, h) {
                Ok(f) => {
                    ok += 1;
                    assert!(border_is_wall(&f.grid));
                    assert_starts(&f);
                    assert_one_area(&f);
                    assert_share(&f.grid);
                }
                Err(e) => {
                    crowded += 1;
                    assert_eq!(e, Refused::Crowded, "plan {i} at {w}x{h}");
                }
            }
        }
    }
    // Most invented plans fit; the check means nothing if all are refused.
    assert!(ok > crowded * 2, "{ok} fitted, {crowded} crowded");
}

#[test]
fn outline_has_doors() {
    let p = plan(vec![shape(ShapeKind::Outline, [0.3, 0.3, 0.7, 0.7])]);
    for (w, h) in [(30, 18), (80, 40), (120, 60)] {
        let f = fit(&p, w, h).unwrap();
        let m = 1 + MARGIN;
        let centre = pos(m + (w - 2 * m) / 2, m + (h - 2 * m) / 2);
        assert!(walls_in_inner_area(&f.grid) > 0, "the outline is drawn");
        let seen = flood(&f.grid, pos(1, 1));
        let i = usize::from(centre.y) * usize::from(w) + usize::from(centre.x);
        assert!(seen[i], "{w}x{h}: the inside is reachable from the lane");
    }
}

#[test]
fn crowded_plan_is_refused() {
    // A 10 by 10 lattice of blocks that covers 70 % of the square.
    let side = 0.7f32.sqrt() / 10.0;
    let mut shapes = Vec::new();
    for i in 0..10 {
        for j in 0..10 {
            let (x, y) = (i as f32 / 10.0, j as f32 / 10.0);
            shapes.push(shape(ShapeKind::Block, [x, y, x + side, y + side]));
        }
    }
    let p = plan(shapes);
    for (w, h) in [(30, 18), (80, 40)] {
        assert_eq!(fit(&p, w, h), Err(Refused::Crowded), "{w}x{h}");
    }
    // At 120x60 each block maps to about 10 by 5 cells; drawn as outlines with doors (sealed
    // insides counted) they stay under the share, so the lattice fits with one free area.
    let f = fit(&p, 120, 60).expect("the hollow lattice fits at 120x60");
    assert_starts(&f);
    assert_one_area(&f);
    assert_share(&f.grid);
}

#[test]
fn too_small_is_refused() {
    assert_eq!(fit(&builtin(), 29, 18), Err(Refused::TooSmall));
    assert_eq!(fit(&builtin(), 30, 17), Err(Refused::TooSmall));
    assert_eq!(fit(&builtin(), 0, 0), Err(Refused::TooSmall));
}

#[test]
fn lines_with_zero_height_are_kept() {
    let empty = fit(&plan(Vec::new()), 80, 40).unwrap();
    assert_eq!(walls_in_inner_area(&empty.grid), 0);
    let flat = plan(vec![shape(ShapeKind::Line, [0.2, 0.5, 0.8, 0.5])]);
    let upright = plan(vec![shape(ShapeKind::Line, [0.5, 0.2, 0.5, 0.8])]);
    for p in [flat, upright] {
        let f = fit(&p, 80, 40).unwrap();
        assert!(walls_in_inner_area(&f.grid) > 0, "{p:?}");
    }
    // A point, or a flat shape of another kind, has no extent.
    let none = plan(vec![
        shape(ShapeKind::Line, [0.5, 0.5, 0.5, 0.5]),
        shape(ShapeKind::Block, [0.2, 0.5, 0.8, 0.5]),
        shape(ShapeKind::Outline, [0.5, 0.2, 0.5, 0.8]),
        shape(ShapeKind::Tower, [1.5, 1.5, 2.0, 2.0]),
    ]);
    assert_eq!(fit(&none, 80, 40), Ok(empty));
}

#[test]
fn every_mapped_shape_covers_a_cell() {
    let tiny = plan(vec![shape(ShapeKind::Tower, [0.5, 0.5, 0.5001, 0.5001])]);
    let f = fit(&tiny, 30, 18).unwrap();
    assert_eq!(walls_in_inner_area(&f.grid), 1);
}

#[test]
fn nan_rects_are_skipped() {
    let empty = fit(&plan(Vec::new()), 80, 40).unwrap();
    let bad = plan(vec![
        shape(ShapeKind::Block, [f32::NAN, 0.1, 0.5, 0.5]),
        shape(ShapeKind::Line, [0.1, 0.5, f32::INFINITY, 0.5]),
        shape(ShapeKind::Outline, [0.1, f32::NEG_INFINITY, 0.5, 0.5]),
        shape(ShapeKind::Tower, [0.1, 0.1, 0.5, f32::NAN]),
    ]);
    assert_eq!(fit(&bad, 80, 40), Ok(empty));
    // Reversed bounds are the same rect.
    let fwd = plan(vec![shape(ShapeKind::Block, [0.2, 0.2, 0.4, 0.3])]);
    let rev = plan(vec![shape(ShapeKind::Block, [0.4, 0.3, 0.2, 0.2])]);
    assert_eq!(fit(&fwd, 80, 40), fit(&rev, 80, 40));
}

#[test]
fn same_plan_same_grid() {
    let mut r = Rng::new(7);
    let shapes = (0..30)
        .map(|i| {
            let kind = [ShapeKind::Line, ShapeKind::Outline, ShapeKind::Tower][i % 3];
            shape(kind, random_rect(&mut r))
        })
        .collect();
    let p = plan(shapes);
    assert_eq!(fit(&p, 97, 43), fit(&p, 97, 43));
    assert_eq!(fit(&builtin(), 80, 40), fit(&builtin(), 80, 40));
}

#[test]
fn blocked_start_is_carved_free() {
    // A block around the user program's target, with no start within 10 cells of it.
    let p = plan(vec![shape(ShapeKind::Block, [0.05, 0.2, 0.4, 0.8])]);
    let f = fit(&p, 120, 60).unwrap();
    assert_eq!(
        f.starts[0],
        (pos(30, 30), Dir::Right),
        "carved at the target"
    );
    assert_eq!(
        f.grid.get(pos(20, 30)),
        Tile::Empty,
        "a corridor back to the lane"
    );
    assert_eq!(f.grid.get(pos(20, 31)), Tile::Wall, "the block stays");
    assert_starts(&f);
    assert_one_area(&f);
}

#[test]
fn shapes_past_the_cap_are_skipped() {
    let empty = fit(&plan(Vec::new()), 80, 40).unwrap();
    let mut shapes = vec![shape(ShapeKind::Line, [f32::NAN; 4]); MAX_SHAPES];
    shapes.push(shape(ShapeKind::Block, [0.2, 0.2, 0.4, 0.4]));
    assert_eq!(fit(&plan(shapes), 80, 40), Ok(empty));

    // 100,000 shapes, lines and blocks over the whole square, stay fast.
    let mut r = Rng::new(3);
    let shapes: Vec<Shape> = (0..100_000)
        .map(|i| {
            if i % 2 == 0 {
                shape(ShapeKind::Block, [0.0, 0.0, 1.0, 1.0])
            } else {
                shape(ShapeKind::Line, random_rect(&mut r))
            }
        })
        .collect();
    let p = plan(shapes);
    let started = Instant::now();
    let fitted = fit(&p, 120, 60);
    let took = started.elapsed();
    match fitted {
        Ok(f) => {
            assert_starts(&f);
            assert_one_area(&f);
            assert_share(&f.grid);
        }
        Err(e) => assert_eq!(e, Refused::Crowded),
    }
    eprintln!("100,000 shapes at 120x60: {took:?}");
    assert!(took.as_secs() < 5, "{took:?}");
}

#[test]
fn shapes_off_the_square_are_skipped() {
    let empty = fit(&plan(Vec::new()), 80, 40).unwrap();
    let off = plan(vec![
        shape(ShapeKind::Line, [1.2, 0.2, 1.5, 0.4]),
        shape(ShapeKind::Line, [-0.5, 0.3, -0.1, 0.3]),
        shape(ShapeKind::Line, [0.2, 1.1, 0.8, 1.1]),
        shape(ShapeKind::Block, [0.2, 1.1, 0.4, 1.3]),
        shape(ShapeKind::Outline, [-0.4, -0.4, -0.2, -0.2]),
        shape(ShapeKind::Tower, [1.01, 0.2, 1.3, 0.6]),
    ]);
    assert_eq!(fit(&off, 80, 40), Ok(empty));
    // A flat line on the edge of the square still overlaps it.
    let edge = plan(vec![shape(ShapeKind::Line, [0.2, 1.0, 0.8, 1.0])]);
    let f = fit(&edge, 80, 40).unwrap();
    assert!(walls_in_inner_area(&f.grid) > 0);
}

#[test]
fn sealed_pockets_count_toward_the_share() {
    // 8 by 5 small outlines without doors: their sides cover about a third of the inner area,
    // with the sealed insides well over 40 %.
    let mut shapes = Vec::new();
    for i in 0..8 {
        for j in 0..5 {
            let (x, y) = (i as f32 * 0.125 + 0.02, j as f32 * 0.2 + 0.02);
            shapes.push(shape(ShapeKind::Outline, [x, y, x + 0.07, y + 0.15]));
        }
    }
    assert_eq!(fit(&plan(shapes.clone()), 80, 40), Err(Refused::Crowded));
    // A few of them fit, with the sealed insides counted.
    shapes.truncate(6);
    let f = fit(&plan(shapes), 80, 40).unwrap();
    assert_share(&f.grid);
    assert_one_area(&f);
}
