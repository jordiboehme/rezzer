// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A region the engine keeps whole is placed whole (Task 31c, class K).
//!
//! A reader hands its text to a walk line by line, and its lines come from a pass that runs
//! before the regions are found: text on one baseline is one line there. A line can so hold the
//! text of two regions: the header cells of a table and, right of the table, a label that
//! stands over its value. [`part_lines`] parts such a line where a block of several lines
//! starts.

use crate::extract::table::CELL_GAP;
use crate::work;

/// The lines of a stretch of runs, with some lines parted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parted {
    /// The line of each run: a number per line or part of a line, counted from 0.
    pub lines: Vec<u32>,
    /// The regions that start at a cut, sorted: a block of text never goes from one of them
    /// into other text.
    pub whole: Vec<u32>,
}

/// Parts the lines of a stretch of runs where a block of several lines starts in the middle of
/// a line. `line` is the line of each run (the runs of a line stand next to each other, the
/// lines in the order they are read), `region` the region the engine decided the run as text
/// (`None` for a run of no such region), `boxed` the drawn box or grid the run stands in, and
/// `inked` whether it has ink. A run without ink goes with the run before it on its line.
/// `several` says of a region that it has ink on more than one line of the text the
/// stretch is a part of, the lines of the stretch among them (the PDF reader passes the
/// regions with ink on more than one line of the page's text).
///
/// A line is cut before a run when all of this holds:
///
/// - the run starts a region the run before it is not of, and the region has ink on another
///   line too: the engine kept a block of several lines whole;
/// - the rest of the line is of that region, and so is the whole next line: the block goes on
///   under the run, alone on its line. Where the next line again holds other text, the two
///   texts stand in rows (a column of labels and a column of values are two regions, and each
///   row is a label with its value): rows are never parted. The last line of the stretch
///   has no next line to ask: there the caller must say `several` of the region (a block
///   kept as laid out stands between its lines, and the stretch may end inside it). As
///   `several` counts the lines of the stretch too, the last line of a block whose other
///   lines stand over it in the stretch is cut off the text of a grid as well, though
///   nothing of the block follows it: the block's own line then stands alone, which reads
///   no text wrong;
/// - the run and the run before it stand in different drawn boxes, or one of them in none: a
///   drawn edge lies between them. Without it a label with a value of two lines beside it
///   looks the same, and stays one line.
///
/// - white space of more than [`CELL_GAP`] text sizes lies between the two runs (`extent`: the
///   left edge, the right edge and the text size of each run): the gap of two cells of a line,
///   not that of two words. A bracket that closes a word of a table's cell touches it.
///
/// Nothing is moved: the block starts on a line of its own, where it stood. `None` when no
/// line is cut.
pub fn part_lines(
    line: &[u32],
    region: &[Option<u32>],
    boxed: &[Option<u32>],
    inked: &[bool],
    extent: &[(f32, f32, f32)],
    several: &dyn Fn(u32) -> bool,
) -> Option<Parted> {
    let n = line.len();
    debug_assert!(region.len() == n && boxed.len() == n && inked.len() == n && extent.len() == n);
    work::add(n as u64);
    // The start of each line, and one past the last.
    let mut starts: Vec<usize> = (0..n)
        .filter(|&i| i == 0 || line[i] != line[i - 1])
        .collect();
    starts.push(n);
    // A cheap first look: no line holds ink of two regions.
    let mixed = |a: usize, b: usize| {
        let mut first: Option<Option<u32>> = None;
        (a..b)
            .filter(|&i| inked[i])
            .any(|i| *first.get_or_insert(region[i]) != region[i])
    };
    if !starts.windows(2).any(|w| mixed(w[0], w[1])) {
        return None;
    }
    // The lines each region has ink on, each once: the pairs of a region and a line, sorted.
    // One sort for all regions, so a stretch of many regions (a page of short paragraphs)
    // costs as much as one of few.
    let mut on: Vec<(u32, usize)> = Vec::new();
    for (l, w) in starts.windows(2).enumerate() {
        on.extend(
            (w[0]..w[1])
                .filter(|&i| inked[i])
                .filter_map(|i| region[i].map(|r| (r, l))),
        );
    }
    on.sort_unstable();
    on.dedup();
    work::add(on.len() as u64);
    // The region has ink on two lines of the stretch, or on more than one line by `several`.
    let lines_of = |r: u32| {
        let k = on.partition_point(|s| s.0 < r);
        on.get(k + 1).is_some_and(|s| s.0 == r) || several(r)
    };
    // All the ink of the runs `a..b` is of region `r`, and there is some.
    let all_of = |a: usize, b: usize, r: u32| {
        let mut inked_runs = (a..b).filter(|&i| inked[i]).peekable();
        inked_runs.peek().is_some() && inked_runs.all(|i| region[i] == Some(r))
    };
    let mut cuts: Vec<usize> = Vec::new();
    let mut whole: Vec<u32> = Vec::new();
    for l in 0..starts.len() - 1 {
        let (a, b) = (starts[l], starts[l + 1]);
        let next_end = starts.get(l + 2).copied();
        let mut before: Option<usize> = None;
        for i in (a..b).filter(|&i| inked[i]) {
            if let (Some(p), Some(r)) = (before, region[i])
                && region[p] != Some(r)
                && boxed[p] != boxed[i]
                && extent[i].0 - extent[p].1 > CELL_GAP * extent[p].2.max(extent[i].2)
                && lines_of(r)
                && all_of(i, b, r)
                && next_end.map_or_else(|| several(r), |end| all_of(b, end, r))
            {
                cuts.push(i);
                whole.push(r);
                break;
            }
            before = Some(i);
        }
    }
    if cuts.is_empty() {
        return None;
    }
    whole.sort_unstable();
    whole.dedup();
    let mut lines = Vec::with_capacity(n);
    let mut at = 0u32;
    for i in 0..n {
        if i > 0 && (line[i] != line[i - 1] || cuts.binary_search(&i).is_ok()) {
            at += 1;
        }
        lines.push(at);
    }
    work::add(n as u64);
    Some(Parted { lines, whole })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parting of runs that all have ink, with no region that has ink outside the stretch.
    fn part(line: &[u32], region: &[Option<u32>], boxed: &[Option<u32>]) -> Option<Parted> {
        part_lines(
            line,
            region,
            boxed,
            &vec![true; line.len()],
            &far(line.len()),
            &|_| false,
        )
    }

    /// Runs of size 10, each 40 wide and 30 after the one before it: three sizes apart.
    fn far(n: usize) -> Vec<(f32, f32, f32)> {
        (0..n)
            .map(|i| (70.0 * i as f32, 70.0 * i as f32 + 40.0, 10.0))
            .collect()
    }

    /// The header cells of a table and a label beside it on one line; the label's value on
    /// the next line. The label's region has two lines and goes on alone under it, and the
    /// header cells stand in the table's grid: the line is cut before the label.
    #[test]
    fn a_label_beside_a_header_row_starts_its_own_line() {
        let line = [0, 0, 0, 0, 1, 2, 3];
        let region = [None, None, None, Some(5), Some(5), Some(6), Some(6)];
        let boxed = [Some(1), Some(1), Some(1), None, None, None, None];
        let p = part(&line, &region, &boxed).unwrap();
        assert_eq!(p.lines, [0, 0, 0, 1, 2, 3, 4]);
        assert_eq!(p.whole, [5]);
        // Each condition alone, the other way: no drawn edge between the header cells and the
        // label (a label with a value of two lines beside it looks the same);
        assert_eq!(part(&line, &region, &[None; 7]), None);
        assert_eq!(part(&line, &region, &[Some(1); 7]), None);
        // the label's region has this one line;
        let one = [None, None, None, Some(5), Some(7), Some(6), Some(6)];
        assert_eq!(part(&line, &one, &boxed), None);
        // the next line holds other text too (rows of two texts);
        let rows = [0, 0, 0, 0, 1, 1, 3];
        let both = [None, None, None, Some(5), None, Some(5), Some(6)];
        assert_eq!(part(&rows, &both, &boxed), None);
        // there is no next line.
        assert_eq!(part(&line[..4], &region[..4], &boxed[..4]), None);
        // the label stands a word gap after the last header cell: `CELL_GAP` sizes of the
        // larger of the two runs or less (10.1 and 9.9 points at a size of 10).
        let gap = |g: f32, size: f32| {
            let mut e = far(7);
            e[3] = (e[2].1 + g, e[2].1 + g + 40.0, size);
            part_lines(&line, &region, &boxed, &[true; 7], &e, &|_| false)
        };
        assert!(gap(10.1, 10.0).is_some());
        assert_eq!(gap(9.9, 10.0), None);
        assert_eq!(gap(10.1, 10.2), None);
        // Two boxes are an edge as well as a box and none.
        let two = [Some(1), Some(1), Some(1), Some(2), Some(2), None, None];
        assert_eq!(
            part(&line, &region, &two).unwrap().lines,
            [0, 0, 0, 1, 2, 3, 4]
        );
    }

    /// A column of labels and a column of values are two regions of several lines, and each
    /// line is a label with its value: rows stay rows, with or without boxes.
    #[test]
    fn rows_of_two_regions_are_never_parted() {
        let line = [0, 0, 1, 1, 2, 2];
        let region = [Some(1), Some(2), Some(1), Some(2), Some(1), Some(2)];
        assert_eq!(part(&line, &region, &[None; 6]), None);
        let boxed = [Some(1), None, Some(1), None, Some(1), None];
        assert_eq!(part(&line, &region, &boxed), None);
        // Nor on the last row, which has no line under it.
        assert_eq!(part(&line[..2], &region[..2], &boxed[..2]), None);
    }

    /// A value on the line of the first line of a wrapped label: the value's region has one
    /// line, and the label's region starts the line. Nothing is cut.
    #[test]
    fn a_value_beside_the_first_line_of_a_label_stays_on_it() {
        let line = [0, 0, 1];
        let region = [Some(1), Some(2), Some(1)];
        let boxed = [None, Some(3), None];
        assert_eq!(part(&line, &region, &boxed), None);
    }

    /// A region with its one line at the end of the stretch and another outside the stretch is
    /// a region of several lines that goes on past the stretch.
    #[test]
    fn a_region_with_a_line_outside_the_stretch_is_one_of_several_lines() {
        let line = [0, 0];
        let region = [None, Some(4)];
        let boxed = [Some(1), None];
        assert_eq!(part(&line, &region, &boxed), None);
        let got = part_lines(&line, &region, &boxed, &[true; 2], &far(2), &|r| r == 4).unwrap();
        assert_eq!((got.lines, got.whole), (vec![0, 1], vec![4]));
        // Review 1 of Task 31c, M4: on the last line of the stretch the caller is asked,
        // and it says `several` of a region with ink on more than one line anywhere, the
        // stretch included (so the PDF reader). A block whose first line stands over this
        // one in the stretch is such a region: its last line is cut off the text of the
        // grid, though the block does not go on under it.
        let line = [0, 1, 1];
        let region = [Some(4), None, Some(4)];
        let boxed = [None, Some(1), None];
        let got = part_lines(&line, &region, &boxed, &[true; 3], &far(3), &|r| r == 4).unwrap();
        assert_eq!((got.lines, got.whole), (vec![0, 1, 2], vec![4]));
        // The other way: a caller that says it of no region cuts no last line.
        assert_eq!(part(&line, &region, &boxed), None);
    }

    /// A run without ink parts nothing, and stays with the run before it.
    #[test]
    fn a_run_without_ink_goes_with_the_run_before_it() {
        let line = [0, 0, 0, 1];
        let region = [None, Some(9), Some(2), Some(2)];
        let boxed = [Some(1), None, None, None];
        let inked = [true, false, true, true];
        let p = part_lines(&line, &region, &boxed, &inked, &far(4), &|_| false).unwrap();
        assert_eq!(p.lines, [0, 0, 1, 2]);
        // A blank run between two runs of one region is no change of region.
        let region = [Some(2), None, Some(2), Some(2)];
        assert_eq!(
            part_lines(&line, &region, &boxed, &inked, &far(4), &|_| false),
            None
        );
    }

    /// The work is linear in the runs: twice the lines take at most a little over twice the
    /// steps, with one region on all lines and with a region of its own on every two lines
    /// (review 1 of Task 31c, M8: the lines of the regions are sorted once, and counted).
    #[test]
    fn the_work_grows_with_the_runs() {
        let steps = |rows: u32, per_region: u32| {
            let line: Vec<u32> = (0..rows).flat_map(|r| [r, r]).collect();
            let region: Vec<Option<u32>> = (0..rows)
                .flat_map(|r| [None, Some(2 + r / per_region)])
                .collect();
            let boxed: Vec<Option<u32>> = (0..rows).flat_map(|_| [Some(1), None]).collect();
            let before = crate::work_steps().unwrap();
            assert_eq!(part(&line, &region, &boxed), None);
            crate::work_steps().unwrap() - before
        };
        for per_region in [u32::MAX, 2] {
            let (a, b) = (steps(200, per_region), steps(400, per_region));
            assert!(b * 10 <= a * 21, "{per_region}: {a} {b}");
            // The lines of the regions are part of the count: a step a run and one a line
            // of a region at least.
            assert!(a >= 400 + 200, "{per_region}: {a}");
        }
    }
}
