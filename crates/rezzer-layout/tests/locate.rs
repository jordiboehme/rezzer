// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The layout detector (`sheetlayout::locate`) and the reader (`rezzer_model::locate_cells`)
//! give every cell to the same region, also when regions overlap.

use rezzer_model::{CellArea, locate_cells};
use sheetlayout::{Pos, Rect, locate};

fn both(rects: &[Rect], points: &[(u32, u32)]) -> Vec<Option<usize>> {
    let areas: Vec<CellArea> = rects
        .iter()
        .map(|r| CellArea {
            top: r.top,
            left: r.left,
            bottom: r.bottom,
            right: r.right,
        })
        .collect();
    let pos: Vec<Pos> = points.iter().map(|&(row, col)| Pos { row, col }).collect();
    let detector = locate(rects, &pos);
    let reader = locate_cells(&areas, points);
    assert_eq!(detector, reader, "rects {rects:?}");
    reader
}

fn rect(top: u32, left: u32, bottom: u32, right: u32) -> Rect {
    Rect {
        top,
        left,
        bottom,
        right,
    }
}

#[test]
fn equal_sizes_go_to_the_upper_left_then_the_lower_index() {
    // Two 2 x 2 areas overlap in B2; two equal areas overlap whole.
    let rects = [
        rect(1, 1, 2, 2),
        rect(0, 0, 1, 1),
        rect(5, 0, 6, 1),
        rect(5, 0, 6, 1),
    ];
    let found = both(&rects, &[(1, 1), (2, 2), (0, 0), (5, 1)]);
    assert_eq!(found, [Some(1), Some(0), Some(1), Some(2)]);
    // The order of the list does not change who holds a point between different areas.
    let swapped = [rects[1], rects[0], rects[2], rects[3]];
    assert_eq!(both(&swapped, &[(1, 1)]), [Some(0)]);
}

#[test]
fn many_overlapping_areas_agree() {
    // An outer area and 65 one-column areas inside it.
    let mut rects = vec![rect(0, 0, 9, 78)];
    for col in 1..=65 {
        rects.push(rect(0, col, 9, col));
    }
    let points: Vec<(u32, u32)> = (0..10).flat_map(|r| (0..80).map(move |c| (r, c))).collect();
    let found = both(&rects, &points);
    assert_eq!(found[4 * 80 + 70], Some(0));
    assert_eq!(found[4 * 80 + 3], Some(3));
    assert_eq!(found[4 * 80 + 79], None);
}

#[test]
fn random_overlapping_areas_agree() {
    // A cheap deterministic generator, so every run checks the same shapes.
    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = |n: u32| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x % u64::from(n)) as u32
    };
    for _ in 0..500 {
        let rects: Vec<Rect> = (0..next(12))
            .map(|_| {
                let (top, left) = (next(10), next(10));
                rect(top, left, top + next(5), left + next(5))
            })
            .collect();
        let points: Vec<(u32, u32)> = (0..16).flat_map(|r| (0..16).map(move |c| (r, c))).collect();
        both(&rects, &points);
    }
}
