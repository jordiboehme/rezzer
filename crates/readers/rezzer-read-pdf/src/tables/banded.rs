// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tables under a header band: a row of drawn cells (filled or framed) with column titles,
//! and below it lines of text without rules that stand in the band's columns.

use super::*;
use crate::regions::Seg;

/// A header band: a grid of drawn cells one row high. `xs` are its cell edges, `b` its box.
#[derive(Debug, Clone)]
pub(super) struct Band {
    pub xs: Vec<f32>,
    pub b: [f32; 4],
}

/// A header band is at most this many times its largest title text high.
const BAND_HEIGHT: f32 = 4.0;

/// Lines below a band at most this many line heights apart belong to its table.
const BODY_GAP: f32 = 3.0;

/// Lines a page's bands may look at together; past them the rest are not looked at.
const BAND_WORK: usize = 4_000_000;

/// What the ruled detector shares with the band search.
pub(super) struct Shared<'a> {
    pub page: u32,
    pub flow: &'a Flow,
    pub lines: &'a [FlowLine],
    pub by_y: &'a ByY,
    pub area: &'a Area<'a>,
    /// The page's horizontal lines, sorted by height.
    pub hs: &'a [Seg],
}

/// What the band search changes: the runs it claims or holds, its refusals and tables.
pub(super) struct Taken<'a> {
    pub claimed: &'a mut [Option<(usize, usize, usize)>],
    pub held: &'a mut [bool],
    pub refusals: &'a mut Vec<(usize, f32, &'static str)>,
    pub tables: &'a mut Vec<Planned>,
    pub skipped: &'a mut Vec<String>,
}

/// The tables under the header bands of a page, top to bottom. A band is at least three of
/// its cells with words; its table is the lines below it down to a rule as wide as the band, a
/// gap of [`BODY_GAP`] line heights or a line whose text does not stand in its columns. A band
/// whose table does not hold is refused with a note.
pub(super) fn banded(sh: &Shared<'_>, mut bands: Vec<Band>, take: &mut Taken<'_>) {
    bands.sort_by(|a, b| b.b[3].total_cmp(&a.b[3]));
    let mut work = 0usize;
    for (k, band) in bands.iter().enumerate() {
        if work > BAND_WORK {
            take.skipped.push(format!(
                "tables under header bands on page {} are looked for only in part: the page \
                 has too many lines below its bands",
                sh.page
            ));
            break;
        }
        // The next band below ends this one's table.
        let floor = bands[k + 1..]
            .iter()
            .filter(|o| o.b[2] > band.b[0] && o.b[0] < band.b[2])
            .map(|o| o.b[3])
            .fold(f32::MIN, f32::max);
        one_band(sh, band, floor, take, &mut work);
    }
}

/// A line of a band's body: its baseline and size, and its pieces, each a column and its
/// runs.
type BodyLine = (f32, f32, Vec<(usize, Vec<usize>)>);

/// The column of `xs` that holds the stretch `x0..x1`, when one does.
fn column_of(xs: &[f32], x0: f32, x1: f32) -> Option<usize> {
    let c = xs
        .partition_point(|e| *e <= (x0 + x1) / 2.0)
        .checked_sub(1)?;
    (c + 1 < xs.len() && x0 >= xs[c] - SNAP && x1 <= xs[c + 1] + SNAP).then_some(c)
}

fn one_band(sh: &Shared<'_>, band: &Band, floor: f32, take: &mut Taken<'_>, work: &mut usize) {
    let (flow, xs, b) = (sh.flow, &band.xs, band.b);
    let cols = xs.len() - 1;
    let free = |i: usize, take: &Taken<'_>| {
        let r = &flow.runs[i];
        !r.off_axis && take.claimed[i].is_none() && !take.held[i]
    };
    // The band's own runs and the cells they stand in.
    let inside = |i: usize| {
        let (cx, _) = middle(&flow.runs[i]);
        cx > b[0] && cx < b[2]
    };
    let head: Vec<usize> = sh
        .by_y
        .runs_between(b[1], b[3])
        .into_iter()
        .filter(|&i| inside(i) && free(i, take))
        .collect();
    let mut titles = vec![String::new(); cols];
    for &i in &head {
        let r = &flow.runs[i];
        let (cx, _) = middle(r);
        let c = xs
            .partition_point(|e| *e < cx)
            .saturating_sub(1)
            .min(cols - 1);
        titles[c].push_str(r.text.trim());
    }
    let titled: Vec<bool> = titles.iter().map(|t| !t.is_empty()).collect();
    // Column titles are words: a row of numbers in boxes is a scale or a counter. A band is
    // a line or two high: a taller row of boxes holds blocks of text.
    let size = head
        .iter()
        .map(|&i| flow.runs[i].size)
        .fold(0.0f32, f32::max);
    if titles
        .iter()
        .filter(|t| !t.is_empty() && !numeric(t))
        .count()
        < 3
        || b[3] - b[1] > BAND_HEIGHT * size
    {
        return;
    }
    // The body: lines below the band, from the top.
    let lo = sh.by_y.line_y.partition_point(|y| *y < b[1]);
    let mut body: Vec<BodyLine> = Vec::new();
    let mut last_y = b[1];
    let mut misfit = false;
    for &k in sh.by_y.lines[..lo].iter().rev() {
        *work += 1;
        let l = &sh.lines[k];
        if l.y <= floor || *work > BAND_WORK {
            break;
        }
        let runs: Vec<usize> = (l.start..l.end)
            .filter(|&i| {
                let r = &flow.runs[i];
                let (cx, _) = middle(r);
                cx > b[0] - SNAP && cx < b[2] + SNAP && free(i, take) && !r.text.trim().is_empty()
            })
            .collect();
        *work += l.end - l.start;
        if runs.is_empty() {
            continue;
        }
        if last_y - l.y > BODY_GAP * l.size.max(1.0) {
            break;
        }
        // A rule as wide as the band between this line and the one above ends the table.
        let top = l.y + l.size;
        let from = sh.hs.partition_point(|s| s.at < top);
        let to = sh.hs.partition_point(|s| s.at < last_y);
        let tol = SNAP.max(0.05 * (b[2] - b[0]));
        *work += to.saturating_sub(from);
        if sh.hs[from..to.max(from)]
            .iter()
            .any(|s| s.from <= b[0] + tol && s.to >= b[2] - tol)
        {
            break;
        }
        // Pieces: runs apart by more than a cell gap; each must stand in one column.
        let mut pieces: Vec<(f32, f32, Vec<usize>)> = Vec::new();
        for &i in &runs {
            let r = &flow.runs[i];
            match pieces.last_mut() {
                Some(p) if r.x - p.1 <= CELL_GAP * r.size.max(1.0) => {
                    p.1 = p.1.max(r.x + r.w);
                    p.2.push(i);
                }
                _ => pieces.push((r.x, r.x + r.w, vec![i])),
            }
        }
        let mut placed: Vec<(usize, Vec<usize>)> = Vec::new();
        for (x0, x1, ids) in pieces {
            match column_of(xs, x0, x1) {
                Some(c) => placed.push((c, ids)),
                None => {
                    misfit = true;
                    break;
                }
            }
        }
        if misfit {
            break;
        }
        body.push((l.y, l.size, placed));
        last_y = l.y;
    }
    let at = head
        .iter()
        .chain(body.iter().flat_map(|l| l.2.iter().flat_map(|p| &p.1)))
        .map(|&i| flow.at[i])
        .min()
        .unwrap_or(0);
    let refuse = |why: &'static str, take: &mut Taken<'_>| {
        take.refusals.push((at, b[3], why));
        for &i in head
            .iter()
            .chain(body.iter().flat_map(|l| l.2.iter().flat_map(|p| &p.1)))
        {
            take.held[i] = true;
        }
    };
    // Rows: lines with text in at least half of the columns the body uses, the first of them
    // included; other lines join the row above them.
    let mut used: Vec<usize> = body.iter().flat_map(|l| l.2.iter().map(|p| p.0)).collect();
    used.sort_unstable();
    used.dedup();
    let Some(&first) = used.first() else {
        let why = if misfit {
            "the text below its header does not stand in its columns"
        } else {
            "too few rows follow its header"
        };
        refuse(why, take);
        return;
    };
    let mut row_of: Vec<usize> = Vec::with_capacity(body.len());
    let mut rows = 0usize;
    for (_, _, placed) in &body {
        let mut c: Vec<usize> = placed.iter().map(|p| p.0).collect();
        c.sort_unstable();
        c.dedup();
        if 2 * c.len() >= used.len() && c.first() == Some(&first) {
            rows += 1;
        }
        row_of.push(rows.saturating_sub(1));
    }
    if rows < 3 {
        refuse("too few rows follow its header", take);
        return;
    }
    // Columns with text in the band or the body; the others are dropped.
    let kept: Vec<usize> = (0..cols)
        .filter(|&c| titled[c] || used.binary_search(&c).is_ok())
        .collect();
    if (rows + 1).saturating_mul(kept.len()) > MAX_SQUARES {
        take.skipped.push(format!(
            "a table of {} by {} cells under a header band on page {} is not read as a table: \
             it has more than {MAX_SQUARES} cells; its text stays paragraphs",
            rows + 1,
            kept.len(),
            sh.page
        ));
        for &i in head
            .iter()
            .chain(body.iter().flat_map(|l| l.2.iter().flat_map(|p| &p.1)))
        {
            take.held[i] = true;
        }
        return;
    }
    // The text of each body cell, for the tests of a table without rules.
    let mut texts: Vec<Vec<String>> = vec![vec![String::new(); cols]; rows];
    for ((_, _, placed), &r) in body.iter().zip(&row_of) {
        for (c, ids) in placed {
            let t = &mut texts[r][*c];
            for &i in ids {
                for w in flow.runs[i].text.split_whitespace() {
                    if !t.is_empty() {
                        t.push(' ');
                    }
                    t.push_str(w);
                }
            }
        }
    }
    if let Some(why) = long_or_prose(&texts) {
        refuse(why, take);
        return;
    }
    let bottom = body.last().map_or(b[1], |l| l.0 - 0.25 * l.1);
    let sizes: Vec<f32> = body.iter().map(|l| l.1).collect();
    if let Some(why) = sh.area.refuses([b[0], bottom, b[2], b[3]], &sizes) {
        // The page's frame, a picture or tiny text: no note, as for a ruled grid.
        let _ = why;
        return;
    }
    if take.tables.len() >= MAX_TABLES {
        take.skipped.push(too_many_tables(sh.page));
        return;
    }
    let t = take.tables.len();
    let slot = |c: usize| kept.binary_search(&c).ok();
    for &i in &head {
        let (cx, _) = middle(&flow.runs[i]);
        let c = xs
            .partition_point(|e| *e < cx)
            .saturating_sub(1)
            .min(cols - 1);
        if let Some(k) = slot(c) {
            take.claimed[i] = Some((t, 0, k));
        }
    }
    for ((_, _, placed), &r) in body.iter().zip(&row_of) {
        for (c, ids) in placed {
            if let Some(k) = slot(*c) {
                for &i in ids {
                    take.claimed[i] = Some((t, r + 1, k));
                }
            }
        }
    }
    let cells = |_: usize| -> Vec<PlannedCell> {
        kept.iter()
            .map(|_| PlannedCell {
                runs: Vec::new(),
                row_span: 1,
                col_span: 1,
            })
            .collect()
    };
    take.tables.push(Planned {
        at,
        top: b[3],
        rows: (0..=rows).map(cells).collect(),
        header_rows: 1,
        trs: Vec::new(),
        additions: Vec::new(),
        tagged: None,
        continues: false,
    });
}

/// Why the cells of a table without rules are not a table's: their text is long, or runs on
/// from a cell into the cell below like prose.
pub(super) fn long_or_prose(texts: &[Vec<String>]) -> Option<&'static str> {
    let mut lens: Vec<usize> = texts
        .iter()
        .flatten()
        .filter(|t| !t.is_empty())
        .map(|t| t.chars().count())
        .collect();
    lens.sort_unstable();
    let numbers = texts.iter().flatten().filter(|t| numeric(t)).count();
    if lens.is_empty() || (lens[lens.len() / 2] >= SHORT_CELL && 2 * numbers < lens.len()) {
        return Some("its cells hold long text");
    }
    for r in 1..texts.len() {
        for (above, below) in texts[r - 1].iter().zip(&texts[r]) {
            let words = above.split_whitespace().count();
            let ends = above.ends_with(['.', ':', ';', '!', '?']);
            let line = words >= PROSE_WORDS && above.chars().count() >= PROSE_LINE;
            if line && !ends && below.chars().next().is_some_and(char::is_lowercase) {
                return Some("its text runs on from row to row like prose");
            }
        }
    }
    None
}
