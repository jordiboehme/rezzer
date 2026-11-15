// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Groups of text lines that keep one style: a step in size, in weight or in colour starts a
//! new group. A line is never parted: its runs in other styles (a bold label, small capitals,
//! a coloured word) are inline styles of the line.

use crate::extract::text::{NAME_ADDRESS_LINES, name_over_address};
use crate::input::TextRun;
use crate::line::{TextLine, ink, line_style, text_line};
use crate::segment::cut::SIZE_STEP;
use crate::work;

/// Splits the runs `ids` (indices into `texts`, in reading order) into groups of whole lines of
/// one style. Consecutive ids of one line number are one line; its style is that of its run
/// with the most ink characters (the first of those on a tie). A line starts a new group when,
/// against the line before it,
///
/// - its size differs by a size step: the larger of the two sizes is at least [`SIZE_STEP`]
///   times the smaller, the ratio the cut uses for a title beside a field list, so that a page
///   printed larger or smaller has the same steps and both agree. Small steps in a chain (each
///   under the ratio against its neighbour) do not add up: a slow drift is no step. Two
///   lines that are both a small label that ends with a colon and its larger value a cell
///   gap after it, in the same two sizes, do not step either: which of the two cells holds
///   more characters (`Sheet: 03.04.2026` over `Pages: 4`) says nothing about the line.
///   Against a line set in one size such a row is set in the size of its label, for the same
///   reason: a line in the size of the values over the rows is no row of theirs;
/// - its weight differs (bold against not bold), unless both lines are rows of cells of both
///   weights apart by a cell gap (a bold label and a plain value, or a plain name and a bold
///   amount): which of its runs is the longer is no step between such rows, while bold words
///   inline in running text still are;
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
    let mut before_mixed = false;
    let mut before_sizes: Option<(f32, f32)> = None;
    let mut known: Option<[u8; 3]> = None;
    // The lines with ink of the group that is open, and where the first of them stands in
    // `lines`: alone in its group it is the line a name over its address would be.
    let (mut inked_lines, mut first_inked) = (0usize, 0usize);
    for (at, line) in lines.iter().enumerate() {
        // Whether the line is a row of cells of both weights: its runs with ink are of both
        // weights (a bold label and its plain value, a plain name and its bold amount) and
        // stand apart by a cell gap somewhere, not bold words inline in running text.
        let mixed = {
            let mut inked: Vec<&TextRun> = line
                .iter()
                .filter_map(|&id| texts.get(id as usize))
                .filter(|r| r.text.chars().any(|c| !c.is_whitespace()))
                .collect();
            inked.sort_by(|a, b| a.x.total_cmp(&b.x));
            let both = inked.iter().any(|r| r.bold) && inked.iter().any(|r| !r.bold);
            both && inked.windows(2).any(|w| {
                w[1].x - (w[0].x + w[0].w) >= crate::extract::table::CELL_GAP * w[1].size.max(1.0)
            })
        };
        // The two sizes of a line that is a row of cells of both: its runs with ink are of
        // two sizes a size step apart, a small label and its larger value.
        let sizes = {
            let mut inked: Vec<&TextRun> = line
                .iter()
                .filter_map(|&id| texts.get(id as usize))
                .filter(|r| r.text.chars().any(|c| !c.is_whitespace()))
                .collect();
            inked.sort_by(|a, b| a.x.total_cmp(&b.x));
            let small = inked.iter().map(|r| r.size).fold(f32::MAX, f32::min);
            let large = inked.iter().map(|r| r.size).fold(f32::MIN, f32::max);
            // The label says it is one: the small run ends with a colon, and the larger
            // run follows it across the gap. Small text beside larger text alone is
            // anything (a side note beside the lines of a form).
            (small > 0.0
                && large >= SIZE_STEP * small
                && inked.windows(2).any(|w| {
                    w[0].text.trim_end().ends_with(':')
                        && w[0].size < SIZE_STEP * small
                        && w[1].size >= SIZE_STEP * small
                        && w[1].x - (w[0].x + w[0].w)
                            >= crate::extract::table::CELL_GAP * w[1].size.max(1.0)
                }))
            .then_some((small, large))
        };
        // Two such rows are set alike when their small sizes are a size and their large
        // sizes too (no size step between them).
        let alike = match (sizes, before_sizes) {
            (Some(a), Some(b)) => {
                let same = |x: f32, y: f32| x.max(y) < SIZE_STEP * x.min(y);
                same(a.0, b.0) && same(a.1, b.1)
            }
            _ => false,
        };
        work::add(2 * line.len() as u64);
        // The run that sets the line's style (see [`line_style`]).
        let Some(run) = line_style(line.iter().filter_map(|&id| texts.get(id as usize))) else {
            // A line without ink has no style to step: it joins the group before it (or starts
            // the first one, which the first ink then joins).
            match groups.last_mut() {
                Some(g) => g.extend(line.iter().copied()),
                None => groups.push(line.clone()),
            }
            continue;
        };
        // Against a line set in one size, a row of a small label and its larger value is
        // also set in the size of its label: the label is the print of the form, the value
        // what is filled in, and which of the two holds more characters says nothing. A line
        // in the size of the values over such rows (`REF: ABC/XYZ   20481` over
        // `Sheet: 03.04.2026`) is no row of theirs. This only adds steps: a line starts a
        // group when its longest run steps against the one of the line before, or when the
        // label size does. A caption in the size of the labels over rows with longer values
        // stays a group of its own.
        let (own, other) = match (sizes, before_sizes) {
            (Some(a), None) => (a.0, before.map_or(0.0, |b| b.size)),
            (None, Some(b)) => (run.size, b.0),
            _ => (run.size, before.map_or(0.0, |b| b.size)),
        };
        let steps = |x: f32, y: f32| x.min(y) > 0.0 && x.max(y) >= SIZE_STEP * x.min(y);
        let starts = before.map_or(groups.is_empty(), |b| {
            ((steps(run.size, b.size) || steps(own, other)) && !alike)
                || (run.bold != b.bold && !(mixed && before_mixed))
                || matches!((run.color, known), (Some(x), Some(y)) if x != y)
        });
        // A name over the smaller lines of its address is the first line of the address: no
        // step parts it from them (see [`name_over_address`]). The name's size has no bound
        // here, on purpose: the steps know no body size. The walk decides by the size
        // (`plain_size` in `extract::text::next_block`): a name much larger than the body
        // (a company's name as the head of a letter) keeps its heading or title there, with
        // the lines of its address as their own paragraph, in one group or in two.
        let starts = starts && !(inked_lines == 1 && opens_address(texts, &lines[first_inked..]));
        known = run.color.or(known);
        match groups.last_mut() {
            Some(g) if !starts => {
                g.extend(line.iter().copied());
                if inked_lines == 0 {
                    first_inked = at;
                }
                inked_lines += 1;
            }
            _ => {
                groups.push(line.clone());
                (inked_lines, first_inked) = (1, at);
            }
        }
        before = Some(run);
        before_mixed = mixed;
        before_sizes = sizes;
    }
    groups
}

/// Whether the first of `lines` (each the ids of one line, a line without ink among them
/// skipped) is a name over the lines of its address, as [`name_over_address`] reads them: at
/// most [`NAME_ADDRESS_LINES`] lines are looked at, so a step asks a bounded number of runs.
fn opens_address(texts: &[TextRun], lines: &[Vec<u32>]) -> bool {
    let runs: Vec<Vec<&TextRun>> = lines
        .iter()
        .map(|l| {
            l.iter()
                .filter_map(|&id| texts.get(id as usize))
                .collect::<Vec<&TextRun>>()
        })
        .filter(|l| l.iter().any(|r| ink(&r.text) > 0))
        .take(NAME_ADDRESS_LINES)
        .collect();
    work::add(runs.iter().map(|l| l.len() as u64).sum());
    let lines: Vec<TextLine<'_, &TextRun>> = runs.iter().map(|l| text_line(l)).collect();
    name_over_address(&lines, 0, 1).is_some()
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

    /// Lines of a small label that ends with a colon and its larger
    /// value a cell gap after it are one group, whichever of the two cells holds more
    /// characters (`Sheet: 03.04.2026` over `Pages: 4`: by its longest run the first line is
    /// set in the size of the values and the second in the size of the labels). Without the
    /// colon, with the value a word space after its label, or with labels of another size,
    /// the step stands.
    #[test]
    fn rows_of_a_small_colon_label_and_a_larger_value_do_not_step() {
        let cell = |text: &str, x: f32, size: f32, line: u32| TextRun {
            text: text.to_string(),
            x,
            w: 0.5 * size * text.chars().count() as f32,
            ..run(size, false, None, line)
        };
        let lines = |first: &str, second: &str, gap: f32, small: f32| {
            let value = |line: u32, x: f32, text: &str| cell(text, x + gap, 10.0, line);
            let a = cell(first, 50.0, 8.0, 0);
            let b = cell(second, 50.0, small, 1);
            let (ax, bx) = (a.x + a.w, b.x + b.w);
            vec![a, value(0, ax, "03.04.2026"), b, value(1, bx, "4")]
        };
        let groups = |texts: &[TextRun]| style_steps(texts, &ids(texts.len())).len();
        assert_eq!(groups(&lines("Sheet:", "Pages:", 20.0, 8.0)), 1);
        assert_eq!(groups(&lines("Sheet", "Pages", 20.0, 8.0)), 2, "no colon");
        // The cell gap is one text size of the value (`CELL_GAP`).
        assert_eq!(groups(&lines("Sheet:", "Pages:", 10.5, 8.0)), 1);
        assert_eq!(
            groups(&lines("Sheet:", "Pages:", 9.5, 8.0)),
            2,
            "a word space"
        );
        // The labels of the two lines are of one size: under the size step (1.08), not at it.
        assert_eq!(groups(&lines("Sheet:", "Pages:", 20.0, 7.5)), 1);
        assert_eq!(
            groups(&lines("Sheet:", "Pages:", 20.0, 7.0)),
            2,
            "other labels"
        );
        // The values of the two lines are of one size too (M5): 10.7 beside 10 is under
        // the size step (10.8), 10.9 is past it. The second line is set by its label (its
        // longer run), the first by its value, so the step between them stands then.
        let values = |second: f32| {
            let mut texts = lines("Sheet:", "Pages:", 20.0, 8.0);
            texts[3].size = second;
            style_steps(&texts, &ids(texts.len())).len()
        };
        assert_eq!((values(10.7), values(10.9)), (1, 2));
        // Against a line set in one size the rows are
        // also set in the size of their labels (8). A line in the size of the values (10)
        // over them is a group of its own, whichever cell of the row under it is the
        // longer. The rule only adds steps: a line in the size of the labels joins rows set
        // by their label (the longer run), and stays apart from a row set by its value, as
        // by the longest runs alone. The step is `SIZE_STEP` (1.08) over 8.
        let rows = |first: (&str, &str), second: (&str, &str)| {
            let mut out = Vec::new();
            for (line, (label, value)) in [first, second].into_iter().enumerate() {
                let l = cell(label, 50.0, 8.0, line as u32);
                let x = l.x + l.w + 20.0;
                out.extend([l, cell(value, x, 10.0, line as u32)]);
            }
            out
        };
        let (short, long) = (("Sheet:", "03.04.2026"), ("Sheets in all:", "4"));
        // A line of one size over the rows (`first` the row under it) and after them.
        let under = |size: f32, first: (&str, &str)| {
            let mut texts = vec![cell("Ref: AB/CD 20481", 50.0, size, 0)];
            for mut r in rows(first, ("Pages:", "4")) {
                r.line += 1;
                texts.push(r);
            }
            style_steps(&texts, &ids(texts.len()))
        };
        let over = |size: f32, last: (&str, &str)| {
            let mut texts = rows(("Pages:", "4"), last);
            texts.push(cell("Ref: AB/CD 20481", 50.0, size, 2));
            style_steps(&texts, &ids(texts.len())).len()
        };
        for row in [short, long] {
            assert_eq!(under(10.0, row), vec![vec![0], vec![1, 2, 3, 4]]);
            assert_eq!(over(10.0, row), 2);
        }
        // The row beside the line is set by its label: the line joins up to the step.
        assert_eq!(under(8.0, long).len(), 1);
        assert_eq!((under(8.6, long).len(), under(8.7, long).len()), (1, 2));
        assert_eq!(
            (over(8.0, long), over(8.6, long), over(8.7, long)),
            (1, 1, 2)
        );
        // The row beside the line is set by its value (the longer run): a caption or a note
        // in the size of the labels is a group of its own, as before the rule.
        assert_eq!(under(8.0, short), vec![vec![0], vec![1, 2, 3, 4]]);
        assert_eq!(over(8.0, short), 2);
        // A line a little under the size of the values (10) joins such a row by its longest
        // run, and the label size parts them all the same: no join the rule takes away.
        assert_eq!((under(9.3, short).len(), over(9.3, short)), (2, 2));
    }

    /// A caption in the size of the labels, without a colon, over
    /// rows of a small colon label and a larger value. Where the values are the longer runs
    /// the caption is a group of its own and the rows stay rows of pairs; so is a note in
    /// that size after them. Where the labels are the longer runs the rows are set in the
    /// size of the caption and it is of their group.
    #[test]
    fn a_caption_in_the_label_size_stays_apart_from_rows_with_longer_values() {
        let cell = |text: &str, x: f32, size: f32, line: u32| TextRun {
            text: text.to_string(),
            x,
            w: 0.5 * size * text.chars().count() as f32,
            ..run(size, false, None, line)
        };
        let groups = |rows: [(&str, &str); 2]| {
            let mut texts = vec![cell("Details of the holder", 50.0, 8.0, 0)];
            for (k, (label, value)) in rows.into_iter().enumerate() {
                let l = cell(label, 50.0, 8.0, 1 + k as u32);
                let x = l.x + l.w + 20.0;
                texts.extend([l, cell(value, x, 10.0, 1 + k as u32)]);
            }
            texts.push(cell("see the notes overleaf", 50.0, 8.0, 3));
            style_steps(&texts, &ids(texts.len()))
        };
        assert_eq!(
            groups([("Name:", "Sample"), ("Date:", "03.04.2026")]),
            vec![vec![0], vec![1, 2, 3, 4], vec![5]]
        );
        assert_eq!(
            groups([("Name of the holder:", "Sample"), ("Date of issue:", "4")]),
            vec![vec![0, 1, 2, 3, 4, 5]]
        );
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

    /// Rows of a plain label and a bold amount apart by a cell gap are
    /// one group, whichever of the two is the longer run of a row; a plain line over them still
    /// steps, and so do lines whose bold words stand inline in their text.
    #[test]
    fn rows_of_both_weights_make_no_weight_step() {
        let cell = |text: &str, bold: bool, x: f32, line: u32| TextRun {
            text: text.into(),
            x,
            w: 5.0 * text.chars().count() as f32,
            ..run(9.0, bold, None, line)
        };
        let texts = vec![
            cell("Fee Summary:", false, 50.0, 0),
            cell("Gross", false, 50.0, 1),
            cell(" 525.98", true, 300.0, 1),
            cell("Room & Services", false, 50.0, 2),
            cell(" 1.90", true, 300.0, 2),
            cell("Taxes", false, 50.0, 3),
            cell(" 1.34", true, 300.0, 3),
        ];
        assert_eq!(
            style_steps(&texts, &ids(7)),
            vec![vec![0], vec![1, 2, 3, 4, 5, 6]]
        );
        // Bold words inline (no cell gap): the longer run still sets the line's weight.
        let texts = vec![
            cell("a plain line with", false, 50.0, 0),
            cell("bold", true, 135.0, 0),
            cell("bold words of the line", true, 50.0, 1),
            cell("and", false, 160.0, 1),
        ];
        assert_eq!(style_steps(&texts, &ids(4)), vec![vec![0, 1], vec![2, 3]]);
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

    /// A bold name over the smaller lines of its address is not parted
    /// from them (see `name_over_address`); a heading that stands farther over them, a name
    /// under other lines of its group, and a title over lines that are no address are.
    #[test]
    fn a_name_stays_with_the_lines_of_its_address() {
        let line = |text: &str, size: f32, bold: bool, n: u32, y: f32| TextRun {
            text: text.to_string(),
            y,
            ..run(size, bold, None, n)
        };
        let page = |name: &str, lead: f32| {
            vec![
                line(name, 10.0, true, 0, 700.0),
                line("Mill Lane 4", 8.0, false, 1, 700.0 - lead),
                line("20481 Oldtown", 8.0, false, 2, 700.0 - lead - 10.4),
            ]
        };
        let groups = |texts: &[TextRun]| style_steps(texts, &ids(texts.len()));
        assert_eq!(groups(&page("Sample Trading", 10.4)), vec![vec![0, 1, 2]]);
        // The same at half and at twice the size.
        for k in [0.5f32, 2.0] {
            let mut texts = page("Sample Trading", 10.4);
            for t in &mut texts {
                t.size *= k;
                t.y *= k;
            }
            assert_eq!(groups(&texts), vec![vec![0, 1, 2]], "{k}");
        }
        // A heading two lines over the address, and a title over lines of one shape.
        assert_eq!(groups(&page("Sample Trading", 20.8)).len(), 2);
        let mut fields = page("Chapter 1", 10.4);
        fields[1].text = "Part 2".into();
        fields[2].text = "Annex 3".into();
        assert_eq!(groups(&fields).len(), 2);
        // A bold line over a complete address that has its own name line is a heading over
        // it: the step stays.
        let mut headed = page("Delivery address", 10.4);
        headed[1].text = "Sample Trading".into();
        headed[2].text = "Mill Lane 4".into();
        headed.push(line("20481 Oldtown", 8.0, false, 3, 700.0 - 31.2));
        assert_eq!(groups(&headed), vec![vec![0], vec![1, 2, 3]]);
        // A bold line over the name in its group: the lines over the address are no name.
        let mut under = vec![line("Sender", 10.0, true, 0, 712.0)];
        for mut t in page("Sample Trading", 10.4) {
            t.line += 1;
            under.push(t);
        }
        assert_eq!(groups(&under), vec![vec![0, 1], vec![2, 3]]);
        // White space before the name is of its group and hides nothing.
        let mut spaced = vec![line(" ", 10.0, false, 0, 712.0)];
        for mut t in page("Sample Trading", 10.4) {
            t.line += 1;
            spaced.push(t);
        }
        assert_eq!(groups(&spaced), vec![vec![0, 1, 2, 3]]);
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
