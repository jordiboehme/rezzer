// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Groups of text runs that keep one style: a step in size, in weight or in colour starts a
//! new group. Not used by a reader yet.

use crate::input::TextRun;
use crate::segment::cut::SIZE_STEP;

/// Splits the runs `ids` (indices into `texts`, in reading order) into groups of one style. A
/// run starts a new group when, against the run before it,
///
/// - its size differs by a size step: the larger of the two sizes is at least [`SIZE_STEP`]
///   times the smaller, the ratio the cut uses for a title beside a field list, so that a page
///   printed larger or smaller has the same steps and both agree. Small steps in a chain (each
///   under the ratio against its neighbour) do not add up: a slow drift is no step;
/// - its weight differs (bold against not bold);
/// - its colour differs from the last colour known before it: a run without a colour
///   (`None`) is not compared and keeps that colour.
///
/// Every id is in exactly one group, in its order. An id past the end of `texts` has no style
/// and stays in the group it follows.
pub fn style_steps(texts: &[TextRun], ids: &[u32]) -> Vec<Vec<u32>> {
    let mut groups: Vec<Vec<u32>> = Vec::new();
    let mut before: Option<&TextRun> = None;
    let mut known: Option<[u8; 3]> = None;
    for &id in ids {
        let Some(run) = texts.get(id as usize) else {
            match groups.last_mut() {
                Some(g) => g.push(id),
                None => groups.push(vec![id]),
            }
            continue;
        };
        let starts = before.is_none_or(|b| {
            (run.size.min(b.size) > 0.0 && run.size.max(b.size) >= SIZE_STEP * run.size.min(b.size))
                || run.bold != b.bold
                || matches!((run.color, known), (Some(x), Some(y)) if x != y)
        });
        known = run.color.or(known);
        match groups.last_mut() {
            Some(g) if !starts => g.push(id),
            _ => groups.push(vec![id]),
        }
        before = Some(run);
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{FontKind, Shift};

    fn run(size: f32, bold: bool, color: Option<[u8; 3]>, line: u32) -> TextRun {
        TextRun {
            text: format!("Zeile {line}"),
            x: 50.0,
            y: 700.0 - 12.0 * line as f32,
            w: 60.0,
            h: size,
            size,
            font: 1,
            font_kind: FontKind::Proportional,
            bold,
            italic: false,
            color,
            shift: Shift::Base,
            line,
            off_axis: false,
            gaps: Vec::new(),
        }
    }

    fn ids(n: usize) -> Vec<u32> {
        (0..n as u32).collect()
    }

    #[test]
    fn style_step_starts_a_group() {
        // A title line of 9 points, bold, over body lines of 7 points: two groups.
        let texts = vec![
            run(9.0, true, None, 0),
            run(7.0, false, None, 1),
            run(7.0, false, None, 2),
            run(7.0, false, None, 3),
        ];
        assert_eq!(style_steps(&texts, &ids(4)), vec![vec![0], vec![1, 2, 3]]);
        // A weight step alone starts a group too.
        let texts = vec![
            run(7.0, true, None, 0),
            run(7.0, false, None, 1),
            run(7.0, false, None, 2),
        ];
        assert_eq!(style_steps(&texts, &ids(3)), vec![vec![0], vec![1, 2]]);
        // A size step alone does.
        let texts = vec![run(9.0, false, None, 0), run(7.0, false, None, 1)];
        assert_eq!(style_steps(&texts, &ids(2)), vec![vec![0], vec![1]]);
    }

    #[test]
    fn colour_band_starts_a_group() {
        let grey = Some([128, 128, 128]);
        let black = Some([0, 0, 0]);
        let texts = vec![
            run(7.0, false, black, 0),
            run(7.0, false, black, 1),
            run(7.0, false, grey, 2),
            run(7.0, false, grey, 3),
        ];
        assert_eq!(style_steps(&texts, &ids(4)), vec![vec![0, 1], vec![2, 3]]);
        // A colour that is not known is not compared: no step.
        let texts = vec![
            run(7.0, false, black, 0),
            run(7.0, false, None, 1),
            run(7.0, false, grey, 2),
        ];
        assert_eq!(style_steps(&texts, &ids(3)), vec![vec![0, 1], vec![2]]);
        let texts = vec![
            run(7.0, false, None, 0),
            run(7.0, false, None, 1),
            run(7.0, false, None, 2),
        ];
        assert_eq!(style_steps(&texts, &ids(3)), vec![vec![0, 1, 2]]);
    }

    #[test]
    fn size_step_is_relative() {
        // The same lines at 0.75 and 1.5 times all sizes: the steps are the same.
        for k in [0.75f32, 1.0, 1.5] {
            let texts = vec![
                run(9.0 * k, false, None, 0),
                run(7.0 * k, false, None, 1),
                run(7.0 * k, false, None, 2),
                // Under a step (7 against 7.4): the same group at every scale.
                run(7.4 * k, false, None, 3),
                // At a step (7.4 against 8.1): its own group at every scale.
                run(8.1 * k, false, None, 4),
            ];
            assert_eq!(
                style_steps(&texts, &ids(5)),
                vec![vec![0], vec![1, 2, 3], vec![4]],
                "{k}"
            );
        }
    }

    #[test]
    fn every_id_stays_in_its_order() {
        let texts = vec![run(7.0, false, None, 0), run(9.0, true, None, 1)];
        // An id past the end has no style: it stays with the group before it.
        let groups = style_steps(&texts, &[1, 0, 5, 0]);
        let flat: Vec<u32> = groups.concat();
        assert_eq!(flat, vec![1, 0, 5, 0]);
        assert!(style_steps(&texts, &[]).is_empty());
    }
}
