// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Groups of text lines that keep one style: a step in size, in weight or in colour starts a
//! new group. A line is never parted: its runs in other styles (a bold label, small capitals,
//! a coloured word) are inline styles of the line.

use crate::input::TextRun;
use crate::line::line_style;
use crate::segment::cut::SIZE_STEP;

/// Splits the runs `ids` (indices into `texts`, in reading order) into groups of whole lines of
/// one style. Consecutive ids of one line number are one line; its style is that of its run
/// with the most ink characters (the first of those on a tie). A line starts a new group when,
/// against the line before it,
///
/// - its size differs by a size step: the larger of the two sizes is at least [`SIZE_STEP`]
///   times the smaller, the ratio the cut uses for a title beside a field list, so that a page
///   printed larger or smaller has the same steps and both agree. Small steps in a chain (each
///   under the ratio against its neighbour) do not add up: a slow drift is no step;
/// - its weight differs (bold against not bold);
/// - its colour differs from the last colour known before it: a line without a colour
///   (`None`) is not compared and keeps that colour.
///
/// A line without ink (white space) has no style: it stays in the group it follows, and white
/// space before the first ink is in the group of that ink.
///
/// Every id is in exactly one group, in its order. An id past the end of `texts` has no line
/// and no style and stays with the line it follows.
pub fn style_steps(texts: &[TextRun], ids: &[u32]) -> Vec<Vec<u32>> {
    // The lines: runs of consecutive ids of one line number.
    let mut lines: Vec<Vec<u32>> = Vec::new();
    let mut line_of: Option<u32> = None;
    for &id in ids {
        match texts.get(id as usize) {
            Some(run) if lines.is_empty() || line_of != Some(run.line) => {
                if line_of.is_none() && !lines.is_empty() {
                    // Ids without a line so far: they open this line.
                    line_of = Some(run.line);
                    if let Some(l) = lines.last_mut() {
                        l.push(id);
                    }
                    continue;
                }
                line_of = Some(run.line);
                lines.push(vec![id]);
            }
            Some(_) => {
                if let Some(l) = lines.last_mut() {
                    l.push(id);
                }
            }
            None => match lines.last_mut() {
                Some(l) => l.push(id),
                None => lines.push(vec![id]),
            },
        }
    }
    let mut groups: Vec<Vec<u32>> = Vec::new();
    let mut before: Option<&TextRun> = None;
    let mut known: Option<[u8; 3]> = None;
    for line in lines {
        // The run that sets the line's style (see [`line_style`]).
        let Some(run) = line_style(line.iter().filter_map(|&id| texts.get(id as usize))) else {
            // A line without ink has no style to step: it joins the group before it (or starts
            // the first one, which the first ink then joins).
            match groups.last_mut() {
                Some(g) => g.extend(line),
                None => groups.push(line),
            }
            continue;
        };
        let starts = before.map_or(groups.is_empty(), |b| {
            (run.size.min(b.size) > 0.0 && run.size.max(b.size) >= SIZE_STEP * run.size.min(b.size))
                || run.bold != b.bold
                || matches!((run.color, known), (Some(x), Some(y)) if x != y)
        });
        known = run.color.or(known);
        match groups.last_mut() {
            Some(g) if !starts => g.extend(line),
            _ => groups.push(line),
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

    /// A run without ink has no style to step: a regular space between two bold runs joins the
    /// group before it, and white space before the first ink joins the group of that ink.
    #[test]
    fn white_space_starts_no_group() {
        let mut space = run(7.0, false, None, 0);
        space.text = " ".into();
        let texts = vec![
            run(7.0, true, None, 0),
            space.clone(),
            run(7.0, true, None, 0),
            run(7.0, false, None, 1),
        ];
        assert_eq!(style_steps(&texts, &ids(4)), vec![vec![0, 1, 2], vec![3]]);
        let texts = vec![space, run(9.0, true, None, 0), run(7.0, false, None, 1)];
        assert_eq!(style_steps(&texts, &ids(3)), vec![vec![0, 1], vec![2]]);
    }

    /// A line is never parted: a bold label and its plain value, small capitals in two sizes,
    /// a coloured word, all stay one line in one group; the line's style is its longest run's.
    #[test]
    fn a_line_keeps_its_inline_styles() {
        let mut label = run(7.0, true, None, 0);
        label.text = "Nr:".into();
        let mut value = run(7.0, false, None, 0);
        value.text = "Zeile mit mehr Zeichen".into();
        let mut caps = run(9.0, false, Some([200, 0, 0]), 0);
        caps.text = "M".into();
        let texts = vec![label, caps, value, run(7.0, false, None, 1)];
        assert_eq!(style_steps(&texts, &ids(4)), vec![vec![0, 1, 2, 3]]);
        // A bold line under them still starts a group, at every scale.
        for k in [0.75f32, 1.0, 1.5] {
            let mut texts = texts.clone();
            for t in &mut texts {
                t.size *= k;
            }
            texts.push(run(7.0 * k, true, None, 2));
            assert_eq!(
                style_steps(&texts, &ids(5)),
                vec![vec![0, 1, 2, 3], vec![4]]
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
