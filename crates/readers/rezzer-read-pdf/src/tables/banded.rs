// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tables under a header band: a row of drawn cells (filled or framed) with column titles,
//! and below it lines of text without rules that stand in the band's columns.

use layoutregions::extract::table::Cells;
use layoutregions::extract::table::banded::{
    BAND_EDGE, BODY_GAP, BodyLine, band_cells, band_titles, body_pieces,
};
use layoutregions::segment::drawn::Line;

use super::*;

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
    pub hs: &'a [Line],
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
    /// Bands refused with a note or a loss: the ids of their runs, and why.
    pub doubts: &'a mut Doubts,
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

fn one_band(sh: &Shared<'_>, band: &Band, floor: f32, take: &mut Taken<'_>, work: &mut usize) {
    let (flow, xs, b) = (sh.flow, &band.xs, band.b);
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
    let titles = band_titles(&flow.runs, &head, xs);
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
            take.doubts
                .push((head.iter().map(|&i| flow.ids[i]).collect(), why));
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
        take.doubts
            .push((runs().map(|&i| flow.ids[i]).collect(), why));
    };
    // Its cells (see `band_cells`), or why there are none.
    let table = match band_cells(&flow.runs, &head, &titles, xs, (&body, misfit)) {
        Cells::Table(table) => table,
        Cells::TooLarge { rows, cols } => {
            take.skipped.push(format!(
                "a table of {rows} by {cols} cells under a header band on page {} is not read \
                 as a table: it has more than {MAX_SQUARES} cells; its text stays paragraphs",
                sh.page
            ));
            hold(take);
            return;
        }
        Cells::Refused(why) => {
            refuse(why, take);
            return;
        }
        Cells::Stopped => return,
    };
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
            take.doubts
                .push((runs().map(|&i| flow.ids[i]).collect(), why));
        }
        return;
    }
    if take.tables.len() >= MAX_TABLES {
        take.skipped.push(too_many_tables(sh.page));
        return;
    }
    let t = take.tables.len();
    claim_cells(take.claimed, t, &table);
    take.tables.push(Planned {
        at,
        top: b[3],
        rows: planned_rows(&table),
        header_rows: table.header_rows,
        trs: Vec::new(),
        additions: Vec::new(),
        tagged: None,
        continues: false,
        caption: Vec::new(),
        found_by: None,
        kind: layoutregions::Kind::Table,
        ids: Vec::new(),
        by: Detector::Banded,
        drawn: Some(b),
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
        // Pieces, each in one column of the band (see `body_pieces`): a line that does not
        // fit ends the body.
        let Some(placed) = body_pieces(&flow.runs, &runs, xs) else {
            misfit = true;
            break;
        };
        body.push((l.y, l.size, placed));
        last_y = l.y;
    }
    (*work <= BAND_WORK).then_some((body, misfit))
}

/// The lines of a page and their index by height, built once for all rows of boxes that
/// [`heads_lines`] looks at.
pub(super) struct Heads {
    lines: Vec<FlowLine>,
    by_y: ByY,
}

impl Heads {
    pub(super) fn new(flow: &Flow) -> Heads {
        let lines = flow_lines(flow);
        let by_y = ByY::new(flow, &lines);
        Heads { lines, by_y }
    }
}

/// Whether the row of boxes `b` with cell edges `xs` heads lines of text below it that stand
/// in its columns, as a header band heads its table (the body rule of [`one_band`]). `hs`
/// are the page's horizontal lines, sorted by height, and `heads` the lines of `flow`.
/// `work` is shared by all rows of the page: `None` once it passes [`BAND_WORK`].
pub(super) fn heads_lines(
    flow: &Flow,
    heads: &Heads,
    hs: &[Line],
    b: [f32; 4],
    xs: &[f32],
    work: &mut usize,
) -> Option<bool> {
    let area = Area {
        width: 0.0,
        height: 0.0,
        images: &[],
        body_size: 0.0,
    };
    let sh = Shared {
        page: 0,
        flow,
        lines: &heads.lines,
        by_y: &heads.by_y,
        area: &area,
        hs,
        stop: &|| false,
    };
    let free = |i: usize| !flow.runs[i].off_axis;
    body_below(&sh, b, xs, f32::MIN, &free, work).map(|(body, _)| !body.is_empty())
}
