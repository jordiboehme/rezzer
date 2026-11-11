// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Captions next to a table or a figure: a line that opens with "Table 3" names a table, one
//! that opens with "Figure 3", "Fig. 3", "Abbildung 3" or "Abb. 3" a figure.

use super::{RunRange, middle};
use crate::line::{LineRun, PIECE_GAP};

/// Lines of a caption paragraph above a table looked at for its start.
pub const CAPTION_LINES: usize = 8;

/// Why a grid or lines next to a figure caption (or holding one) are no table: a chart's.
pub const FIGURE_NEAR: &str = "a figure caption stands next to it";

/// The largest text size a caption search assumes for a table, in points.
pub const MAX_CAPTION_SIZE: f32 = 36.0;

/// The caption kind of a line: `Some(true)` for "Table 3", `Some(false)` for "Figure 3",
/// "Fig. 3", "Abbildung 3" or "Abb. 3".
pub fn caption(text: &str) -> Option<bool> {
    let t = text.trim_start();
    let (table, rest) = match t.strip_prefix("Table") {
        Some(r) => (true, r),
        None => (
            false,
            t.strip_prefix("Figure")
                .or_else(|| t.strip_prefix("Fig."))
                .or_else(|| t.strip_prefix("Abbildung"))
                .or_else(|| t.strip_prefix("Abb."))?,
        ),
    };
    let rest = rest.trim_start();
    let first = rest.chars().next()?;
    (first.is_ascii_digit()
        || (first.is_ascii_uppercase()
            && rest.len() > 1
            && rest[1..].starts_with(['.', ':', ' ', '-'])))
    .then_some(table)
}

/// Whether the runs `within` (the text of a box) hold a figure caption: a run that opens as one
/// ("Figure 3", "Abb. 3", see [`caption`]) and says more than the figure's name and number in
/// its piece of the line (with the runs that go on from it within [`PIECE_GAP`] sizes: caption
/// words), or is the only run with text of its line among them. A cell that holds only "Abb.
/// 3" next to other cells of its row points to a figure from a table (a parts list, a list of
/// drawings); it captions nothing.
pub fn caption_inside<R: LineRun>(within: &[R]) -> bool {
    within.iter().enumerate().any(|(k, r)| {
        if caption(r.text()) != Some(false) {
            return false;
        }
        let mut line: Vec<&R> = within
            .iter()
            .enumerate()
            .filter(|(j, o)| *j != k && o.line() == r.line() && !o.text().trim().is_empty())
            .map(|(_, o)| o)
            .collect();
        if line.is_empty() {
            return true;
        }
        line.sort_by(|a, b| a.x().total_cmp(&b.x()));
        let mut words = r.text().split_whitespace().count();
        let mut end = r.x() + r.w();
        for o in line.into_iter().filter(|o| o.x() >= r.x()) {
            if o.x() - end > PIECE_GAP * r.size().max(o.size()) {
                break;
            }
            words += o.text().split_whitespace().count();
            end = end.max(o.x() + o.w());
        }
        words > 2
    })
}

/// The runs and lines of a page sorted by height, so a box finds what lies in it by search.
#[derive(Debug, Clone, Default)]
pub struct ByY {
    /// Run indices by the height of their middle, and those heights.
    pub runs: Vec<usize>,
    pub run_y: Vec<f32>,
    /// Line indices by baseline, and the baselines.
    pub lines: Vec<usize>,
    pub line_y: Vec<f32>,
}

impl ByY {
    pub fn new<R: LineRun>(runs: &[R], lines: &[RunRange]) -> ByY {
        // Built once per page: a builder per row or per table would show here.
        crate::work::add((runs.len() + lines.len()) as u64);
        let mut by: Vec<(f32, usize)> = runs
            .iter()
            .enumerate()
            .map(|(i, r)| (middle(r).1, i))
            .collect();
        by.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut ls: Vec<(f32, usize)> = lines.iter().enumerate().map(|(k, l)| (l.y, k)).collect();
        ls.sort_by(|a, b| a.0.total_cmp(&b.0));
        ByY {
            run_y: by.iter().map(|r| r.0).collect(),
            runs: by.into_iter().map(|r| r.1).collect(),
            line_y: ls.iter().map(|l| l.0).collect(),
            lines: ls.into_iter().map(|l| l.1).collect(),
        }
    }

    /// The number of runs whose middle lies between the heights `lo` and `hi`.
    pub fn count_between(&self, lo: f32, hi: f32) -> usize {
        let a = self.run_y.partition_point(|y| *y <= lo);
        let z = self.run_y.partition_point(|y| *y < hi);
        z.saturating_sub(a)
    }

    /// The runs whose middle lies between the heights `lo` and `hi`, in page order.
    pub fn runs_between(&self, lo: f32, hi: f32) -> Vec<usize> {
        let a = self.run_y.partition_point(|y| *y <= lo);
        let z = self.run_y.partition_point(|y| *y < hi);
        let mut out = self.runs[a..z].to_vec();
        out.sort_unstable();
        out
    }
}

/// The caption kinds next to a box: the line of a table caption (the first found), and
/// whether a figure caption is near. A caption counts when its paragraph ends within three
/// line heights above the box, or starts within three line heights below it, and overlaps
/// it across.
/// Lines `skip` names (the box's own) are not looked at. `sizes` are the font sizes of the
/// box's text: their median sets how far from the box a caption may stand.
pub fn captions_near<R: LineRun>(
    runs: &[R],
    lines: &[RunRange],
    by_y: &ByY,
    b: [f32; 4],
    sizes: &[f32],
    skip: &dyn Fn(usize) -> bool,
) -> (Option<usize>, bool) {
    let (mut table, mut figure) = (None, false);
    let text = |l: &RunRange| -> String { runs[l.start..l.end].iter().map(|r| r.text()).collect() };
    let across = |l: &RunRange| {
        let rs = &runs[l.start..l.end];
        let x0 = rs.iter().map(|r| r.x()).fold(f32::MAX, f32::min);
        let x1 = rs.iter().map(|r| r.x() + r.w()).fold(f32::MIN, f32::max);
        !rs.iter().any(|r| r.off_axis()) && x1 >= b[0] && x0 <= b[2]
    };
    let mut note = |k: usize| match caption(&text(&lines[k])) {
        Some(true) => {
            table.get_or_insert(k);
        }
        Some(false) => figure = true,
        None => {}
    };
    // Only lines in a band around the box can be near it: a caption is set about as large as
    // the table's own text.
    let mut sorted = sizes.to_vec();
    sorted.sort_by(f32::total_cmp);
    let size = sorted
        .get(sorted.len() / 2)
        .copied()
        .unwrap_or(10.0)
        .clamp(1.0, MAX_CAPTION_SIZE);
    let band = 4.0 * size;
    let lo = by_y.line_y.partition_point(|y| *y < b[1] - band);
    let hi = by_y.line_y.partition_point(|y| *y <= b[3] + band);
    crate::work::add(1 + hi.saturating_sub(lo) as u64);
    for &k in &by_y.lines[lo..hi] {
        let l = &lines[k];
        if skip(l.start) || !across(l) {
            continue;
        }
        let near = 3.0 * l.size.min(2.0 * size);
        if l.y < b[1] && b[1] - (l.y + l.size) <= near {
            note(k);
        } else if l.y > b[3] && l.y - b[3] <= near {
            // The last line of a paragraph above: its first lines may hold the caption.
            note(k);
            let mut at = k;
            for _ in 0..CAPTION_LINES {
                let Some(prev) = at.checked_sub(1).map(|p| &lines[p]) else {
                    break;
                };
                let gap = prev.y - lines[at].y;
                if gap <= 0.0 || gap > 1.6 * prev.size.max(lines[at].size) || !across(prev) {
                    break;
                }
                note(at - 1);
                at -= 1;
            }
        }
    }
    (table, figure)
}
