// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tables under a header band: a row of drawn cells (filled or framed) with column titles,
//! and below it lines of text without rules that stand in the band's columns.

use std::collections::BTreeMap;

use super::*;
use crate::regions::Seg;

/// A header band: a grid of drawn cells one row high. `xs` are its cell edges, `b` its box.
/// `cut` is set for a section of a grid that was cut in sections: refused, it is a loss.
#[derive(Debug, Clone)]
pub(super) struct Band {
    pub xs: Vec<f32>,
    pub b: [f32; 4],
    pub cut: bool,
}

/// A header band is at most this many times its largest title text high.
const BAND_HEIGHT: f32 = 4.0;

/// Lines below a band at most this many line heights apart belong to its table.
const BODY_GAP: f32 = 3.0;

/// Lines a page's bands may look at together; past them the rest are not looked at.
const BAND_WORK: usize = 4_000_000;

/// A rule ends a band's table when it is as wide as the band, short of it at each end by at
/// most this share of the band's width (or [`SNAP`]).
pub(super) const BAND_EDGE: f32 = 0.05;

/// What the ruled detector shares with the band search.
pub(super) struct Shared<'a> {
    pub page: u32,
    pub flow: &'a Flow,
    pub lines: &'a [FlowLine],
    pub by_y: &'a ByY,
    pub area: &'a Area<'a>,
    /// The page's horizontal lines, sorted by height.
    pub hs: &'a [Seg],
    pub stop: &'a dyn Fn() -> bool,
}

/// What the band search changes: the runs it claims or holds, its refusals and tables.
pub(super) struct Taken<'a> {
    pub claimed: &'a mut [Option<(usize, usize, usize)>],
    pub held: &'a mut [bool],
    pub refusals: &'a mut Vec<(usize, f32, &'static str)>,
    /// Bands refused without a note, for the losses of the page, and their runs.
    pub quiet: &'a mut Vec<(usize, &'static str, Refused)>,
    pub quietly: &'a mut [bool],
    pub tables: &'a mut Vec<Planned>,
    pub skipped: &'a mut Vec<String>,
}

/// The tables under the header bands of a page, top to bottom. A band is at least three of
/// its cells with words; its table is the lines below it down to a rule as wide as the band, a
/// gap of [`BODY_GAP`] line heights or a line whose text does not stand in its columns. A band
/// whose table does not hold is refused with a note.
pub(super) fn banded(sh: &Shared<'_>, mut bands: Vec<Band>, take: &mut Taken<'_>) {
    bands.sort_by(|a, b| b.b[3].total_cmp(&a.b[3]));
    // A band is at most BAND_HEIGHT times the largest text of the page high: a taller one
    // is let go before its text is looked at.
    let largest = sh.flow.runs.iter().map(|r| r.size).fold(0.0f32, f32::max);
    let mut work = 0usize;
    for (k, band) in bands.iter().enumerate() {
        if k % CHECK_EVERY == CHECK_EVERY - 1 && (sh.stop)() {
            take.skipped.push(format!(
                "tables under header bands on page {} are looked for only in part: the time \
                 limit was reached",
                sh.page
            ));
            break;
        }
        // A section of a cut grid is judged by its own text below.
        if !band.cut && band.b[3] - band.b[1] > BAND_HEIGHT * largest {
            continue;
        }
        if work > BAND_WORK {
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
    // Once the budget is spent, also in the last band, one note says so; the bands it cut
    // are left as text.
    if work > BAND_WORK {
        take.skipped.push(format!(
            "tables under header bands on page {} are looked for only in part: the page has \
             too much text in and below its bands",
            sh.page
        ));
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
    // The runs in the band's height are counted (a search) and charged before they are
    // copied.
    let in_height = sh.by_y.run_y.partition_point(|y| *y < b[3])
        - sh.by_y.run_y.partition_point(|y| *y <= b[1]);
    *work += in_height;
    if *work > BAND_WORK {
        return;
    }
    let head: Vec<usize> = sh
        .by_y
        .runs_between(b[1], b[3])
        .into_iter()
        .filter(|&i| inside(i) && free(i, take))
        .collect();
    // The titles of the cells that hold text, by column.
    let mut titles: BTreeMap<usize, String> = BTreeMap::new();
    for &i in &head {
        let r = &flow.runs[i];
        let (cx, _) = middle(r);
        let c = xs
            .partition_point(|e| *e < cx)
            .saturating_sub(1)
            .min(cols - 1);
        titles.entry(c).or_default().push_str(r.text.trim());
    }
    titles.retain(|_, t| !t.is_empty());
    // Column titles are words: a row of numbers in boxes is a scale or a counter. A band is
    // a line or two high: a taller row of boxes holds blocks of text.
    let size = head
        .iter()
        .map(|&i| flow.runs[i].size)
        .fold(0.0f32, f32::max);
    let tall = b[3] - b[1] > BAND_HEIGHT * size;
    if titles.values().filter(|t| !numeric(t)).count() < 3 || tall {
        // A section of a cut grid with text is not let go without a word.
        if band.cut
            && let Some(at) = head.iter().map(|&i| flow.at[i]).min()
        {
            let why = if tall {
                "it is too tall for a header over lines"
            } else {
                "its cells hold too few column titles"
            };
            take.quiet.push((at, why, Refused::Band));
            for &i in &head {
                take.quietly[i] = true;
            }
        }
        return;
    }
    // The body: lines below the band, from the top.
    let free_now = |i: usize| free(i, take);
    let Some((body, misfit)) = body_below(sh, b, xs, floor, &free_now, work) else {
        // The budget ran out in this body: no table and no refusal, the note says why.
        return;
    };
    if *work > BAND_WORK {
        return;
    }
    // The runs of the band's titles and body.
    let runs = || {
        head.iter()
            .chain(body.iter().flat_map(|l| l.2.iter().flat_map(|p| &p.1)))
    };
    let at = runs().map(|&i| flow.at[i]).min().unwrap_or(0);
    // A refused band holds them as paragraphs.
    let hold = |take: &mut Taken<'_>| {
        for &i in runs() {
            take.held[i] = true;
        }
    };
    let refuse = |why: &'static str, take: &mut Taken<'_>| {
        take.refusals.push((at, b[3], why));
        hold(take);
    };
    // Rows: see [`row_starts`].
    let mut used: Vec<usize> = body.iter().flat_map(|l| l.2.iter().map(|p| p.0)).collect();
    used.sort_unstable();
    used.dedup();
    if used.is_empty() {
        let why = if misfit {
            "the text below its header does not stand in its columns"
        } else {
            "too few rows follow its header"
        };
        refuse(why, take);
        return;
    }
    let mut row_of: Vec<usize> = Vec::with_capacity(body.len());
    let mut rows = 0usize;
    let pieces: Vec<Vec<(usize, bool)>> = body
        .iter()
        .map(|(_, _, placed)| {
            placed
                .iter()
                .map(|(c, ids)| {
                    let text: String = ids.iter().map(|&i| flow.runs[i].text.as_str()).collect();
                    (*c, numeric(&text))
                })
                .collect()
        })
        .collect();
    for starts in row_starts(&pieces, used.len()) {
        rows += usize::from(starts);
        row_of.push(rows.saturating_sub(1));
    }
    if rows < 3 {
        refuse("too few rows follow its header", take);
        return;
    }
    // Columns with text in the band or the body; the others are dropped.
    let mut kept: Vec<usize> = titles.keys().copied().chain(used.iter().copied()).collect();
    kept.sort_unstable();
    kept.dedup();
    if (rows + 1).saturating_mul(kept.len()) > MAX_SQUARES {
        take.skipped.push(format!(
            "a table of {} by {} cells under a header band on page {} is not read as a table: \
             it has more than {MAX_SQUARES} cells; its text stays paragraphs",
            rows + 1,
            kept.len(),
            sh.page
        ));
        hold(take);
        return;
    }
    // The text of each body cell, for the tests of a table without rules: only the kept
    // columns, so the table is never as wide as all of the band's cells.
    let slot = |c: usize| kept.binary_search(&c).ok();
    let mut texts: Vec<Vec<String>> = vec![vec![String::new(); kept.len()]; rows];
    for ((_, _, placed), &r) in body.iter().zip(&row_of) {
        for (c, ids) in placed {
            let Some(k) = slot(*c) else { continue };
            let t = &mut texts[r][k];
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
    let area = [b[0], bottom, b[2], b[3]];
    if let Some(why) = sh.area.refuses(area, &sizes) {
        // The page's frame or tiny text: no note and its text stays free, as for a ruled
        // grid, but a loss. On a picture the band is a chart's: no loss either.
        if !sh.area.on_picture(area) {
            take.quiet.push((at, why, Refused::Band));
            for &i in runs() {
                take.quietly[i] = true;
            }
        }
        return;
    }
    if take.tables.len() >= MAX_TABLES {
        take.skipped.push(too_many_tables(sh.page));
        return;
    }
    let t = take.tables.len();
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
                status: None,
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
        caption: Vec::new(),
        found_by: None,
    });
}

/// The lines below the band `b` with cell edges `xs` that are its body, from the top: lines
/// of runs `free` takes, above `floor`, each less than [`BODY_GAP`] line heights below the
/// one before, with no rule as wide as the band between them, each piece of each standing in
/// one column. Also whether a line that does not stand in the columns ended it. `None` once
/// `work` passes [`BAND_WORK`].
fn body_below(
    sh: &Shared<'_>,
    b: [f32; 4],
    xs: &[f32],
    floor: f32,
    free: &dyn Fn(usize) -> bool,
    work: &mut usize,
) -> Option<(Vec<BodyLine>, bool)> {
    let flow = sh.flow;
    let lo = sh.by_y.line_y.partition_point(|y| *y < b[1]);
    let mut body: Vec<BodyLine> = Vec::new();
    let mut last_y = b[1];
    let mut misfit = false;
    for &k in sh.by_y.lines[..lo].iter().rev() {
        *work += 1;
        let l = &sh.lines[k];
        if l.y <= floor {
            break;
        }
        if *work > BAND_WORK {
            return None;
        }
        let runs: Vec<usize> = (l.start..l.end)
            .filter(|&i| {
                let r = &flow.runs[i];
                let (cx, _) = middle(r);
                cx > b[0] - SNAP && cx < b[2] + SNAP && free(i) && !r.text.trim().is_empty()
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
        let tol = SNAP.max(BAND_EDGE * (b[2] - b[0]));
        *work += to.saturating_sub(from);
        if sh.hs[from..to.max(from)]
            .iter()
            .any(|s| s.from <= b[0] + tol && s.to >= b[2] - tol)
        {
            break;
        }
        // Pieces: runs apart by more than a cell gap, or by a column edge of the band in the
        // white space between them (a value line padded into the band's columns is cut there,
        // see `cut_at_columns`); each must stand in one column.
        let inner = &xs[1..xs.len() - 1];
        let mut pieces: Vec<(f32, f32, Vec<usize>)> = Vec::new();
        for &i in &runs {
            let r = &flow.runs[i];
            let tol = 0.1 * r.size.max(1.0);
            match pieces.last_mut() {
                Some(p)
                    if r.x - p.1 <= CELL_GAP * r.size.max(1.0)
                        && !inner.iter().any(|&e| e >= p.1 - tol && e <= r.x + tol) =>
                {
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
    (*work <= BAND_WORK).then_some((body, misfit))
}

/// Whether the row of boxes `b` with cell edges `xs` heads lines of text below it that stand
/// in its columns, as a header band heads its table (the body rule of [`one_band`]). `hs`
/// are the page's horizontal lines, sorted by height.
pub(super) fn heads_lines(flow: &Flow, hs: &[Seg], b: [f32; 4], xs: &[f32]) -> bool {
    let lines = flow_lines(flow);
    let by_y = ByY::new(flow, &lines);
    let area = Area {
        width: 0.0,
        height: 0.0,
        images: &[],
        body_size: 0.0,
    };
    let sh = Shared {
        page: 0,
        flow,
        lines: &lines,
        by_y: &by_y,
        area: &area,
        hs,
        stop: &|| false,
    };
    let mut work = 0;
    let free = |i: usize| !flow.runs[i].off_axis;
    body_below(&sh, b, xs, f32::MIN, &free, &mut work).is_some_and(|(body, _)| !body.is_empty())
}

/// For the lines of a body without rules from the top, each its pieces as (column, whether
/// the piece is a number), whether the line starts a row. A line that fills at least half of
/// the `used` columns starts one, also with an empty first cell (a date printed once a day).
/// A line that fills less goes on the row above only when each of its pieces goes on text in
/// its column there and none is a number (a cell that wraps); any other line is a row of its
/// own, with empty cells where it has no piece. Lines of words above the first line that
/// fills half of the columns, each in a column that line fills, are the first lines of its
/// cells (a cell that wraps upward over values set at the middle of the row): they start no
/// row and go on the first row. Any other first line starts a row.
pub(super) fn row_starts(lines: &[Vec<(usize, bool)>], used: usize) -> Vec<bool> {
    let columns = |pieces: &[(usize, bool)]| {
        let mut cols: Vec<usize> = pieces.iter().map(|p| p.0).collect();
        cols.sort_unstable();
        cols.dedup();
        cols
    };
    let full = |pieces: &[(usize, bool)]| 2 * columns(pieces).len() >= used;
    // The lines above the first full line that wrap upward into its cells.
    let lead = lines
        .iter()
        .position(|l| full(l))
        .filter(|&f| {
            let below = columns(&lines[f]);
            lines[..f]
                .iter()
                .flatten()
                .all(|(c, number)| !number && below.contains(c))
        })
        .unwrap_or(0);
    let mut filled: Vec<usize> = lines[..lead].iter().flat_map(|l| columns(l)).collect();
    let mut out = vec![false; lead];
    for (k, pieces) in lines.iter().enumerate().skip(lead) {
        let cols = columns(pieces);
        let joins = k > lead
            && 2 * cols.len() < used
            && pieces
                .iter()
                .all(|(c, number)| !number && filled.contains(c));
        if !joins && k > lead {
            filled.clear();
        }
        filled.extend(cols);
        out.push(!joins);
    }
    out
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
