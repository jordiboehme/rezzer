// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The built-in plan, the Grid.

use super::{Plan, Shape, ShapeKind};

/// The Grid: four towers at the quarter points, a box in the centre and two short walls that
/// lead into it.
pub fn builtin() -> Plan {
    let tower = |cx: f32, cy: f32| Shape {
        kind: ShapeKind::Tower,
        rect: [cx - 0.04, cy - 0.06, cx + 0.04, cy + 0.06],
    };
    let line = |x0: f32, x1: f32| Shape {
        kind: ShapeKind::Line,
        rect: [x0, 0.5, x1, 0.5],
    };
    Plan {
        label: "the Grid".to_string(),
        shapes: vec![
            tower(0.25, 0.25),
            tower(0.75, 0.25),
            tower(0.25, 0.75),
            tower(0.75, 0.75),
            Shape {
                kind: ShapeKind::Outline,
                rect: [0.35, 0.35, 0.65, 0.65],
            },
            line(0.35, 0.42),
            line(0.58, 0.65),
        ],
    }
}
