// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tables from grids of drawn rules.

use std::collections::{HashMap, HashSet};

use layoutregions::extract::pairs::Field;
use layoutregions::extract::table::ruled::{
    COLUMN_CUT_WORK, GridPart, RuledIn, boxed_field, column_cuts, edges, ruled_cells,
};
use layoutregions::extract::table::sections::{CUT_WORK, sections};
use layoutregions::extract::table::{Cells, TITLE_WORK, column_across, title_over};
use layoutregions::segment::drawn::{Grids, Line, Unsearched};

use super::banded::{Band, Shared, Taken, banded};
use super::pairs::{
    Top, claim, field_cuts, field_strips, frame_tops, frames, holds_values, pairs_table,
    taken_by_grids,
};
use super::side::{Beside, SIDE_FOUND_BY, SIDE_WORK, Side, StripKind, side_columns, totals_top};
use super::*;

/// Where a run a table took goes: the table, its row and its cell.
type Claim = Option<(usize, usize, usize)>;

/// A section of a grid: its segments, whether its grid was cut, whether a cut is its bottom
/// or its top, and its grid's place among the page's grids.
struct Part {
    h: Vec<Line>,
    v: Vec<Line>,
    cut: bool,
    cut_below: bool,
    cut_above: bool,
    id: usize,
}

/// The one note of a page for its grids refused with a note: a grid alone says why; several
/// say how many and why, by reason, the reasons in the order they first came. The note stands
/// where the first of them does.
fn page_note(page: u32, refusals: &[(usize, f32, &'static str)]) -> Option<Unsure> {
    let first = refusals.iter().min_by_key(|r| r.0)?;
    let text = (refusals.len() > 1).then(|| {
        format!(
            "Possible tables on page {page} not rebuilt: {}",
            counted(refusals.iter().map(|r| r.2))
        )
    });
    Some(Unsure {
        at: first.0,
        top: first.1,
        reason: first.2.to_string(),
        text,
    })
}

/// Grids of drawn rules: horizontal and vertical lines that meet, with text in their cells.
/// The page's runs are cut at the edges of its fields and columns (see [`cut_runs`]); the
/// columns of totals beside its sections are read first (see [`totals_tables`]), then each
/// section of each grid top to bottom (see [`section`]), the header bands it left (see
/// [`banded`]) and the rows of fields between rules no grid takes (see [`strip_tables`]);
/// last the runs of all tables leave the flow (see [`leave_flow`]). `rules` are the page's
/// drawn rules and `grids` the grids they make.
pub(super) fn ruled(
    page: u32,
    flow: &mut Flow,
    (rules, grids, segmented): (&[Rule], Result<&Grids, Unsearched>, &[bool]),
    area: &Area<'_>,
    stop: &dyn Fn() -> bool,
    found: &mut Found,
) {
    let Grids { hs, vs, groups } = match grids {
        Ok(g) => g,
        Err(Unsearched::TooMany { .. }) => {
            found.skipped.push(format!(
                "ruled tables on page {page} are not looked for: the page draws more than \
                 {MAX_SEGMENTS} straight lines"
            ));
            return;
        }
        Err(Unsearched::Work) => {
            found.skipped.push(format!(
                "ruled tables on page {page} are not looked for: its lines meet too often"
            ));
            return;
        }
    };
    let grids = grid_parts(page, hs, vs, groups, segmented, found);
    // The horizontal lines a grid takes; the others may bound a column of totals or a strip
    // of fields.
    let taken = taken_by_grids(hs.len(), groups);
    let frames = cut_runs(
        page,
        flow,
        (hs, vs, &frame_tops(hs, vs, groups, true)),
        &grids,
        found,
    );
    let lines = flow_lines(flow);
    let by_y = ByY::new(&flow.runs, &lines);
    let sh = Sheet {
        page,
        flow,
        lines: &lines,
        by_y: &by_y,
        area,
        rules,
        hs,
        vs,
        frames: &frames,
        taken: &taken,
        per_line: table::texted_per_line(&flow.runs),
        stop,
    };
    let mut take = Taking::new(flow.runs.len());
    // A column of totals beside the sections: its totals are pairs, taken first; its other
    // strips join the section beside them, below.
    let sides = totals_tables(
        page,
        flow,
        &by_y,
        (hs, vs, &taken),
        &grids,
        (&mut take.claimed, &mut take.tables),
        found,
    );
    // The tables of totals come first; they have no header row.
    let pairs = take.tables.len();
    for (g, part) in grids.into_iter().enumerate() {
        if g % CHECK_EVERY == CHECK_EVERY - 1 && stop() {
            found.skipped.push(format!(
                "ruled tables on page {page} are looked for only in part: the time limit was \
                 reached"
            ));
            break;
        }
        if section(&sh, &sides, part, &mut take, found) == Next::Stop {
            break;
        }
    }
    if take.title_work > TITLE_WORK {
        found.skipped.push(format!(
            "titles over ruled grids on page {page} are looked for only in part: the page has \
             too many column lines"
        ));
    }
    let strips_from = bands_and_strips(&sh, &mut take, found);
    leave_flow(page, flow, take, (pairs, strips_from), found);
}

/// What the grid search of a page reads: the page, its runs, lines and rules, and the frames
/// of fields between its rules.
struct Sheet<'a> {
    page: u32,
    flow: &'a Flow,
    lines: &'a [FlowLine],
    by_y: &'a ByY,
    area: &'a Area<'a>,
    rules: &'a [Rule],
    /// The page's horizontal and vertical lines, joined.
    hs: &'a [Line],
    vs: &'a [Line],
    frames: &'a [Frame],
    /// The horizontal lines a grid takes.
    taken: &'a [bool],
    per_line: HashMap<u32, u32>,
    stop: &'a dyn Fn() -> bool,
}

/// What the grid search of a page takes and finds as it goes.
struct Taking {
    /// Runs a grid took, and for each the table and the row and cell it goes to.
    claimed: Vec<Claim>,
    tables: Vec<Planned>,
    /// Runs of grids refused with a note, and those grids: where each stands and why; and
    /// the grids refused without a note.
    held: Vec<bool>,
    refusals: Vec<(usize, f32, &'static str)>,
    quiet: Vec<(usize, &'static str, Refused)>,
    /// Runs of grids and bands refused without a note: their loss speaks for them.
    quietly: Vec<bool>,
    /// Runs that title the section below a cut: no section above takes them, and the section
    /// below, by its grid, has them as its caption once it is a table.
    titles: Vec<bool>,
    pending: HashMap<usize, Vec<usize>>,
    title_work: usize,
    bands: Vec<Band>,
    /// Cells drawn as icons, the page's rule boxes sorted by their bottom once needed, and
    /// the steps spent looking.
    icons: Vec<(usize, usize, usize, [f32; 4])>,
    strokes: Option<Vec<[f32; 4]>>,
    icon_work: usize,
    /// The grids and bands refused with a note or a loss: the ids of their runs, and why.
    doubts: Doubts,
}

impl Taking {
    /// Nothing taken yet of a flow of `n` runs.
    fn new(n: usize) -> Taking {
        Taking {
            claimed: vec![None; n],
            tables: Vec::new(),
            held: vec![false; n],
            refusals: Vec::new(),
            quiet: Vec::new(),
            quietly: vec![false; n],
            titles: vec![false; n],
            pending: HashMap::new(),
            title_work: 0,
            bands: Vec::new(),
            icons: Vec::new(),
            strokes: None,
            icon_work: 0,
            doubts: Vec::new(),
        }
    }
}

/// Whether the grid search goes on with the next section or stops: the time limit was
/// reached, or the page has as many tables as it may.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Next {
    Section,
    Stop,
}

/// A section of a grid ready to be read as a table: its segments, its column and row edges
/// (from the top), the heights of its open rows (see [`hanging_rows`]), the runs of its title
/// and whether a column of totals joins it.
struct GridSection {
    h: Vec<Line>,
    v: Vec<Line>,
    xs: Vec<f32>,
    ys: Vec<f32>,
    open: Vec<f32>,
    title: Vec<usize>,
    side: bool,
}

/// One section of a grid (see [`grid_parts`]): its edges (see [`section_edges`]), its title,
/// its open rows and the column of totals it joins. A row of drawn cells goes to the header
/// bands, a section of two rows and two columns or more is read (see [`read_grid`]).
fn section(
    sh: &Sheet<'_>,
    sides: &[Side],
    part: Part,
    take: &mut Taking,
    found: &mut Found,
) -> Next {
    let Part {
        mut h,
        v,
        cut,
        cut_below,
        cut_above,
        id,
    } = part;
    let (mut xs, mut ys) = section_edges(sh, &h, &v, cut_above);
    let mut title = take.pending.remove(&id).unwrap_or_default();
    if xs.len() >= 2 && ys.len() >= 2 {
        let mut place = TitlePlace {
            flow: sh.flow,
            by_y: sh.by_y,
            claimed: &take.claimed,
            titles: &mut take.titles,
            xs: &xs,
            work: &mut take.title_work,
        };
        // The first section of a cut grid has the line right over the grid as its title,
        // as each section after it has the line right over its cut.
        if cut && !cut_above {
            title.extend(place.over_grid(sh.vs, ys[0]));
        }
        // The line right above a cut is the title of the section below: it leaves this
        // section, and a row of one cell that held only it goes too.
        if cut_below {
            let below = place.above_cut(&v, &mut ys, &mut h);
            take.pending.insert(id, below);
        }
    }
    let open = hanging_rows(sh, take, &v, &xs, &mut ys, &mut h, cut);
    // A strip of a column of totals beside the section, headed on the baseline of its
    // column titles, is one more column of it, parted at each of its row edges.
    let side = joined(sides, &xs, &ys);
    if let Some(side) = side {
        let right = xs[xs.len() - 1];
        xs.push(side.x1);
        for &y in &ys {
            h.push(Line {
                at: y,
                from: right,
                to: side.x1,
            });
        }
    }
    let (cols, rows) = (xs.len().saturating_sub(1), ys.len().saturating_sub(1));
    if rows == 1 && cols >= 2 {
        // A row of drawn cells may head a table whose lines have no rules.
        take.bands.push(Band {
            b: [xs[0], ys[1], xs[cols], ys[0]],
            xs,
            cut,
        });
        return Next::Section;
    }
    if cols < 2 || rows < 2 {
        return Next::Section;
    }
    if rows.saturating_mul(cols) > MAX_SQUARES {
        found.skipped.push(format!(
            "a grid of {rows} by {cols} drawn cells on page {} is not read as a table: it has \
             more than {MAX_SQUARES} cells",
            sh.page
        ));
        return Next::Section;
    }
    read_grid(
        sh,
        take,
        found,
        GridSection {
            h,
            v,
            xs,
            ys,
            open,
            title,
            side: side.is_some(),
        },
    )
}

/// The column and row edges (from the top) of the section of segments `h` and `v`, cut at
/// rows of fields beyond its rules and, below a cut (`cut_above`), at its own top rule.
fn section_edges(sh: &Sheet<'_>, h: &[Line], v: &[Line], cut_above: bool) -> (Vec<f32>, Vec<f32>) {
    let (mut xs, mut ys) = edges(h, v);
    // Column rules that hang below the grid's lowest rule into a row of fields under it
    // (a frame that holds fields with a value) part the fields, not rows of the grid: the
    // grid ends at that rule. So do column rules that rise over its highest rule into a
    // row of fields over it. Column titles or values over or under the rule are the
    // grid's own open row.
    if let (Some(low), Some(high), Some(&l), Some(&r)) = (
        h.iter().map(|s| s.at).reduce(f32::min),
        h.iter().map(|s| s.at).reduce(f32::max),
        xs.first(),
        xs.last(),
    ) {
        let framed = |at: f32, edge: fn(&Frame) -> f32| {
            sh.frames.iter().any(|f| {
                (edge(f) - at).abs() <= SNAP
                    && f.x0 < r
                    && l < f.x1
                    && holds_values(sh.flow, sh.by_y, f)
            })
        };
        if framed(low, |f| f.top) {
            ys.retain(|y| *y >= low - SNAP);
        }
        if framed(high, |f| f.bottom) {
            ys.retain(|y| *y <= high + SNAP);
        }
        // A row beyond the grid's lowest or highest rule that only the ends of column rules
        // bound and that holds no text is no row of it.
        let empty = |lo: f32, hi: f32| {
            !sh.by_y.runs_between(lo, hi).into_iter().any(|i| {
                let run = &sh.flow.runs[i];
                let (cx, _) = middle(run);
                cx > l && cx < r && !run.text.trim().is_empty()
            })
        };
        if ys.len() >= 3 && ys[0] > high + SNAP && empty(high, ys[0]) {
            ys.remove(0);
        }
        let n = ys.len();
        if n >= 3 && ys[n - 1] < low - SNAP && empty(ys[n - 1], low) {
            ys.pop();
        }
    }
    // A section below a cut reaches only as far left as its own top rule: where another
    // block draws the rule at that height over its left part (fields beside it), the
    // text under that part is the other block's.
    if cut_above && let (Some(&top), Some(&l), Some(&r)) = (ys.first(), xs.first(), xs.last()) {
        let from = own_left(h, sh.hs, top, (l, r));
        if from > xs[0] {
            xs.retain(|x| *x >= from - SNAP);
            if xs.first().is_none_or(|x| *x > from + SNAP) {
                xs.insert(0, from);
            }
        }
    }
    (xs, ys)
}

/// The open rows of a section (see `layoutregions::extract::table::ruled::hanging_rows`), over
/// the free runs of the page that are text.
fn hanging_rows(
    sh: &Sheet<'_>,
    take: &Taking,
    v: &[Line],
    xs: &[f32],
    ys: &mut Vec<f32>,
    h: &mut Vec<Line>,
    cut: bool,
) -> Vec<f32> {
    let holds_text = |bottom: f32, foot: f32| {
        sh.by_y.runs_between(bottom, foot).into_iter().any(|i| {
            let r = &sh.flow.runs[i];
            let (cx, cy) = middle(r);
            !r.off_axis
                && take.claimed[i].is_none()
                && !take.titles[i]
                && cy < foot
                && cx > xs[0]
                && cx < xs[xs.len() - 1]
                && !r.text.trim().is_empty()
        })
    };
    layoutregions::extract::table::ruled::hanging_rows(v, xs, (ys, h), cut, &holds_text)
}

/// How a grid refused is said: a grid that looks like a table (`noted`) gets a note and holds
/// its text as paragraphs; any other grid is a loss only, unless it is a chart's (`chart`),
/// whatever reason refuses it: its text stays free for the detector without rules, as such a
/// grid often frames a table of its own. A figure caption speaks against a table: no note and
/// no loss for it.
struct Refusal<'a> {
    at: usize,
    top: f32,
    noted: bool,
    chart: bool,
    /// The grid's runs, and the ids of the page's runs (see [`Flow::ids`]).
    mine: &'a [usize],
    ids: &'a [u32],
}

impl Refusal<'_> {
    fn refuse(&self, take: &mut Taking, why: &'static str) {
        if why == FIGURE_NEAR {
            return;
        }
        if !self.noted {
            if !self.chart {
                take.quiet.push((self.at, why, Refused::Grid));
                for &i in self.mine {
                    take.quietly[i] = true;
                }
                take.doubts
                    .push((self.mine.iter().map(|&i| self.ids[i]).collect(), why));
            }
            return;
        }
        take.refusals.push((self.at, self.top, why));
        take.doubts
            .push((self.mine.iter().map(|&i| self.ids[i]).collect(), why));
        for &i in self.mine {
            take.held[i] = true;
        }
    }
}

/// Reads a section of two rows and two columns or more as a table, or refuses it (see
/// [`Refusal`]): what the page says against a table (boxed fields, a figure caption, too many
/// columns, the area) is looked at here, the cells, the header and the tests of the cells in
/// the engine (see [`ruled_cells`]); the runs of a table are claimed for its cells, and a run
/// the cells do not place keeps its text where it stands, with a loss.
fn read_grid(sh: &Sheet<'_>, take: &mut Taking, found: &mut Found, s: GridSection) -> Next {
    let (flow, xs, ys) = (sh.flow, &s.xs, &s.ys);
    let (cols, rows) = (xs.len() - 1, ys.len() - 1);
    let b = [xs[0], ys[rows], xs[cols], ys[0]];
    let inside = |i: usize| {
        let r = &flow.runs[i];
        let (cx, _) = middle(r);
        !r.off_axis && take.claimed[i].is_none() && !take.titles[i] && cx > b[0] && cx < b[2]
    };
    let mine: Vec<usize> = sh
        .by_y
        .runs_between(b[1], b[3])
        .into_iter()
        .filter(|&i| inside(i))
        .collect();
    let within: Vec<usize> = mine
        .iter()
        .copied()
        .filter(|&i| !flow.runs[i].text.trim().is_empty())
        .collect();
    if within.is_empty() || boxed_field(&flow.runs, &within, xs, ys) {
        return Next::Section;
    }
    if take.tables.len() >= MAX_TABLES {
        found.skipped.push(too_many_tables(sh.page));
        return Next::Stop;
    }
    let at = flow.at[within[0]];
    let sizes: Vec<f32> = within.iter().map(|&i| flow.runs[i].size).collect();
    let (caption_line, figure_caption) =
        captions_near(&flow.runs, sh.lines, sh.by_y, b, &sizes, &|i| {
            let (_, cy) = middle(&flow.runs[i]);
            inside(i) && cy > b[1] && cy < b[3]
        });
    let table_caption = caption_line.is_some();
    // A refused grid gets a note when a caption names a table, or when it looks like one:
    // three columns or more with text in two rows or in six squares, and nothing about
    // the area (the page's frame, a picture, tiny text) that rules out a table. Its text
    // then stays paragraphs and no detector without rules takes it.
    let mut text_rows: HashSet<usize> = HashSet::new();
    let mut text_squares: HashSet<(usize, usize)> = HashSet::new();
    for &i in &within {
        let (cx, cy) = middle(&flow.runs[i]);
        let r = ys
            .partition_point(|e| *e > cy)
            .saturating_sub(1)
            .min(rows - 1);
        let c = xs
            .partition_point(|e| *e < cx)
            .saturating_sub(1)
            .min(cols - 1);
        text_rows.insert(r);
        text_squares.insert((r, c));
    }
    // A grid over most of the page is its frame, unless text fills at least half of its
    // squares in three rows and three columns or more: a table as large as the page.
    let dense = dense(rows, cols, text_squares.len());
    let noted = table_caption
        || (cols >= 3
            && (text_rows.len() >= 2 || text_squares.len() >= 6)
            && match sh.area.refuses(b, &sizes) {
                None => true,
                Some(why) => why == PAGE_FRAME && dense,
            });
    let refusal = Refusal {
        at,
        top: b[3],
        noted,
        chart: figure_caption || sh.area.on_picture(b),
        mine: &mine,
        ids: &flow.ids,
    };
    // A figure caption next to the grid or inside it is looked at first, so a wide chart
    // gets no note; a grid of many columns is refused before its cells are worked out.
    if figure_caption
        || within
            .iter()
            .any(|&i| caption(&flow.runs[i].text) == Some(false))
    {
        refusal.refuse(take, FIGURE_NEAR);
        return Next::Section;
    }
    if cols > MAX_COLUMNS {
        refusal.refuse(take, MANY_COLUMNS);
        return Next::Section;
    }
    let per_line = &sh.per_line;
    let open = &s.open;
    let cells = ruled_cells(
        &flow.runs,
        &RuledIn {
            h: &s.h,
            v: &s.v,
            xs,
            ys,
            open,
            mine: &mine,
            within: &within,
            per_line,
        },
        sh.stop,
        &|| sh.area.refuses(b, &sizes),
        &mut |cell| drawn_inside(&mut take.strokes, sh.rules, cell, &mut take.icon_work),
    );
    let table = match cells {
        Cells::Table(table) => table,
        Cells::Stopped => {
            found.skipped.push(format!(
                "ruled tables on page {} are looked for only in part: the time limit was reached",
                sh.page
            ));
            return Next::Stop;
        }
        Cells::TooLarge { rows, cols } => {
            found.skipped.push(format!(
                "a grid of {rows} by {cols} cells on page {} is not read as a table: it has \
                 more than {MAX_SQUARES} cells; its text stays paragraphs",
                sh.page
            ));
            // No detector without rules takes its text either.
            for &i in &mine {
                take.held[i] = true;
            }
            return Next::Section;
        }
        Cells::Refused(why) => {
            refusal.refuse(take, why);
            return Next::Section;
        }
    };
    let t = take.tables.len();
    claim_cells(&mut take.claimed, t, &table);
    // A run the cells do not place keeps its text where it stands, and no detector after
    // this one takes it into a table.
    if !table.unplaced.is_empty() {
        for &(i, _) in &table.unplaced {
            take.held[i] = true;
        }
        found.uncertain.push(format!(
            "text in the table on page {} that starts with \"{}\" runs over the side of its \
             cell into the text of another cell: it is read as a paragraph next to the table",
            sh.page,
            first_words(&flow.runs, &table)
        ));
    }
    for &(r, k, cell) in &table.icons {
        take.icons.push((t, r, k, cell));
    }
    for &i in &s.title {
        take.claimed[i] = Some((t, CAPTION_ROW as usize, 0));
    }
    let at = s.title.iter().map(|&i| flow.at[i]).fold(at, usize::min);
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
        found_by: s.side.then_some(SIDE_FOUND_BY),
        kind: if s.side {
            layoutregions::Kind::LabelValue
        } else {
            layoutregions::Kind::Table
        },
        ids: Vec::new(),
        by: Detector::Ruled,
        drawn: Some(b),
    });
    Next::Section
}

/// The tables under the header bands the sections left (see [`banded`]), then the rows of
/// fields between rules no grid takes, in the text no table took (see [`strip_tables`]).
/// Where the tables of the strips start among the page's tables.
fn bands_and_strips(sh: &Sheet<'_>, take: &mut Taking, found: &mut Found) -> usize {
    // Regions without rules never run across a band.
    found.cuts = take.bands.iter().flat_map(|b| [b.b[1], b.b[3]]).collect();
    found.cuts.sort_by(f32::total_cmp);
    banded(
        &Shared {
            page: sh.page,
            flow: sh.flow,
            lines: sh.lines,
            by_y: sh.by_y,
            area: sh.area,
            hs: sh.hs,
            stop: sh.stop,
        },
        std::mem::take(&mut take.bands),
        &mut Taken {
            claimed: &mut take.claimed,
            held: &mut take.held,
            refusals: &mut take.refusals,
            quiet: &mut take.quiet,
            quietly: &mut take.quietly,
            tables: &mut take.tables,
            skipped: &mut found.skipped,
            doubts: &mut take.doubts,
        },
    );
    // Rows of fields between rules no grid takes, in the text no table took.
    let strips_from = take.tables.len();
    let busy: Vec<bool> = (0..sh.flow.runs.len())
        .map(|i| take.claimed[i].is_some() || take.held[i] || take.titles[i])
        .collect();
    let tabled: Vec<bool> = take.claimed.iter().map(Option::is_some).collect();
    let table_rules: Vec<Line> = sh
        .hs
        .iter()
        .zip(sh.taken)
        .filter(|(_, t)| **t)
        .map(|(s, _)| *s)
        .collect();
    strip_tables(
        sh.page,
        sh.flow,
        sh.by_y,
        sh.frames,
        (sh.hs, &table_rules),
        &busy,
        &tabled,
        &mut take.claimed,
        &mut take.held,
        &mut take.tables,
        found,
    );
    strips_from
}

/// The runs of all tables of `take` leave the flow at once, each to its cell; the runs held
/// stay where they are. Then the icons go to their cells, and the first row of each table but
/// those of totals and of strips (`pairs..strips_from`) is judged as a header by its text.
fn leave_flow(
    page: u32,
    flow: &mut Flow,
    mut take: Taking,
    (pairs, strips_from): (usize, usize),
    found: &mut Found,
) {
    read_apart(flow, &mut take, found);
    let Taking {
        claimed,
        mut tables,
        held,
        refusals,
        quiet,
        quietly,
        icons,
        doubts,
        ..
    } = take;
    found.quiet = quiet;
    // The rows of fields refused before (see [`strip_tables`]) said theirs already.
    found.doubts.extend(doubts);
    if let Some(u) = page_note(page, &refusals) {
        found.unsure.push(u);
    }
    if tables.is_empty() {
        if held.contains(&true) {
            found.held = held;
        }
        if quietly.contains(&true) {
            found.quietly = quietly;
        }
        return;
    }
    // The runs of all grids leave the flow at once; the runs held stay where they are.
    let left = |marks: &[bool]| -> Vec<bool> {
        if !marks.contains(&true) {
            return Vec::new();
        }
        marks
            .iter()
            .zip(&claimed)
            .filter(|(_, c)| c.is_none())
            .map(|(h, _)| *h)
            .collect()
    };
    found.held = left(&held);
    found.quietly = left(&quietly);
    if !found.bodies.is_empty() {
        found.bodies = found
            .bodies
            .iter()
            .zip(&claimed)
            .filter(|(_, c)| c.is_none())
            .map(|(b, _)| *b)
            .collect();
    }
    let target: Vec<Target> = claimed
        .into_iter()
        .map(|c| c.map(|(t, r, k)| (t as u32, r as u32, k as u32)))
        .collect();
    fill(&mut tables, flow, &target);
    if let Some(template) = tables
        .iter()
        .flat_map(|t| &t.rows)
        .flatten()
        .find_map(|c| c.runs.first())
        .cloned()
    {
        for (t, r, k, cell) in icons {
            let size = template.size;
            tables[t].rows[r][k].runs.push(Run {
                text: ICON.into(),
                x: cell[0] + SNAP,
                y: (cell[1] + cell[3]) / 2.0 - size / 3.0,
                w: cell[2] - cell[0] - 2.0 * SNAP,
                bold: false,
                italic: false,
                mcid: None,
                uri: None,
                ..template.clone()
            });
        }
    }
    for (k, mut t) in tables.into_iter().enumerate() {
        if t.header_rows == 0 && k >= pairs && k < strips_from {
            t.header_rows = header_of(&t.rows);
        }
        found.tables.push(t);
    }
}

/// Puts the runs of each row of fields of `found.apart` in the flow in the order to read them,
/// cell by cell; other runs of the places of the flow from the first of them to the last (a
/// block beside the row, on its lines) follow them in their own order. The places stay, so the
/// rest of the page reads as before, and what is known of each run (the table cell it goes to,
/// a hold) moves with it. A cell's runs on a line that an earlier cell of the row also has get a
/// line of their own, so the two cells are never read as one line, and each cell is a block of
/// its own (see [`crate::layout::Slot::Block`]), so no paragraph joins two cells; each table
/// whose runs moved stands where its first run now stands.
fn read_apart(flow: &mut Flow, take: &mut Taking, found: &mut Found) {
    fn permute<T: Clone>(v: &mut [T], lo: usize, order: &[usize]) {
        if v.len() < lo + order.len() {
            return;
        }
        let moved: Vec<T> = order.iter().map(|&i| v[i].clone()).collect();
        for (k, x) in moved.into_iter().enumerate() {
            v[lo + k] = x;
        }
    }
    let rows = std::mem::take(&mut found.apart);
    if rows.is_empty() {
        return;
    }
    // Where each run of the flow as it was stands now.
    let mut now: Vec<usize> = (0..flow.runs.len()).collect();
    let mut fresh = flow.runs.iter().map(|r| r.line).max().unwrap_or(0);
    let mut block = 0usize;
    let mut moved_tables: Vec<usize> = Vec::new();
    for cells in rows {
        let cells: Vec<Vec<usize>> = cells
            .into_iter()
            .map(|c| c.into_iter().map(|i| now[i]).collect())
            .collect();
        let mut order: Vec<usize> = cells.iter().flatten().copied().collect();
        let (Some(&lo), Some(&hi)) = (order.iter().min(), order.iter().max()) else {
            continue;
        };
        let mut sorted = order.clone();
        sorted.sort_unstable();
        sorted.dedup();
        if sorted.len() != order.len() {
            continue;
        }
        let rest: Vec<usize> = (lo..=hi)
            .filter(|i| sorted.binary_search(i).is_err())
            .collect();
        order.extend(&rest);
        // Lines of a cell (or of the runs after the row) that an earlier cell of the row has
        // too.
        let mut owned: Vec<u32> = Vec::new();
        let mut lines: Vec<(usize, u32)> = Vec::new();
        for cell in cells.iter().chain(std::iter::once(&rest)) {
            let mine: Vec<u32> = cell.iter().map(|&i| flow.runs[i].line).collect();
            let mut renamed: Vec<(u32, u32)> = Vec::new();
            for (&i, &l) in cell.iter().zip(&mine) {
                if owned.contains(&l) {
                    let n = match renamed.iter().find(|r| r.0 == l) {
                        Some(r) => r.1,
                        None => {
                            fresh += 1;
                            renamed.push((l, fresh));
                            fresh
                        }
                    };
                    lines.push((i, n));
                }
            }
            owned.extend(mine);
        }
        for (i, n) in lines {
            flow.runs[i].line = n;
        }
        for &i in &order {
            if let Some((t, _, _)) = take.claimed[i]
                && !moved_tables.contains(&t)
            {
                moved_tables.push(t);
            }
        }
        // The flow index each original run moves to.
        let mut back: Vec<(usize, usize)> = Vec::new();
        for (k, &i) in order.iter().enumerate() {
            back.push((i, lo + k));
        }
        for n in now.iter_mut() {
            if (lo..=hi).contains(n)
                && let Some(&(_, to)) = back.iter().find(|b| b.0 == *n)
            {
                *n = to;
            }
        }
        permute(&mut flow.runs, lo, &order);
        permute(&mut flow.slots, lo, &order);
        permute(&mut flow.ids, lo, &order);
        // Each cell is a block of its own: its lines go on no line of the next cell.
        let mut k = lo;
        for cell in &cells {
            for slot in &mut flow.slots[k..k + cell.len()] {
                if *slot == crate::layout::Slot::Free {
                    *slot = crate::layout::Slot::Block(block);
                }
            }
            block += 1;
            k += cell.len();
        }
        permute(&mut take.claimed, lo, &order);
        permute(&mut take.held, lo, &order);
        permute(&mut take.quietly, lo, &order);
        permute(&mut take.titles, lo, &order);
        permute(&mut found.bodies, lo, &order);
    }
    for t in moved_tables {
        let first = (0..flow.runs.len()).find(|&i| take.claimed[i].is_some_and(|c| c.0 == t));
        if let (Some(i), Some(table)) = (first, take.tables.get_mut(t)) {
            // The pieces of a run cut at a field edge share its place: a run before the table
            // that shares the place of the table's first run stays before it.
            let shared = i > 0 && flow.at[i - 1] == flow.at[i];
            table.at = flow.at[i] + usize::from(shared);
        }
    }
}

/// Each connected grid of `groups` (of the page's lines `hs` and `vs`) that the ruled detector
/// reads (`segmented`, see `tables::ruled_groups`) in its sections, each
/// judged as a grid of its own, top to bottom; a section of a grid that was cut is no grid's to
/// drop without a word.
fn grid_parts(
    page: u32,
    hs: &[Line],
    vs: &[Line],
    groups: &[(Vec<usize>, Vec<usize>)],
    segmented: &[bool],
    found: &mut Found,
) -> Vec<Part> {
    let mut cut_work = 0usize;
    let mut grids: Vec<Part> = Vec::new();
    for (id, (gh, gv)) in groups.iter().enumerate() {
        if !segmented.get(id).copied().unwrap_or(false) {
            continue;
        }
        let h: Vec<Line> = gh.iter().map(|&j| hs[j]).collect();
        let v: Vec<Line> = gv.iter().map(|&k| vs[k]).collect();
        let parts = sections(&h, &v, hs, &mut cut_work);
        let (cut, last) = (parts.len() > 1, parts.len().saturating_sub(1));
        grids.extend(parts.into_iter().enumerate().map(|(k, (h, v))| Part {
            h,
            v,
            cut,
            cut_below: k < last,
            cut_above: k > 0,
            id,
        }));
    }
    if cut_work > CUT_WORK {
        found.skipped.push(format!(
            "ruled grids on page {page} are cut in sections only in part: the page has too many \
             pieces of lines at the heights of their rules"
        ));
    }
    grids.sort_by(|a, b| {
        let top = |g: &Part| g.h.iter().map(|s| s.at).fold(f32::MIN, f32::max);
        top(b).total_cmp(&top(a))
    });
    grids
}

/// Cuts the runs of the page at the edges of the fields between rules (the frames under the
/// horizontal lines `hs` that `tops` names, see `frame_tops`, with their dividers among `vs`),
/// at a word gap, and at the column lines of the sections `grids`. The frames found.
fn cut_runs(
    page: u32,
    flow: &mut Flow,
    (hs, vs, tops): (&[Line], &[Line], &[Top]),
    grids: &[Part],
    found: &mut Found,
) -> Vec<Frame> {
    let frames = frames(hs, tops, vs);
    if frames.is_none() {
        found.skipped.push(format!(
            "label and value fields between rules on page {page} are not looked for: the page \
             has too many lines"
        ));
    }
    let mut frames = frames.unwrap_or_default();
    // A frame under a grid's lowest rule is a row of fields only when its text holds fields
    // with a value: text under a table's last rule is not.
    if frames.iter().any(|f| f.under_grid) {
        let lines = flow_lines(flow);
        let by_y = ByY::new(&flow.runs, &lines);
        frames.retain(|f| !f.under_grid || holds_values(flow, &by_y, f));
    }
    if !frames.is_empty() {
        apply_cuts(flow, field_cuts(flow, &frames));
    }
    if !cut_at_columns(flow, grids, hs, COLUMN_CUT_WORK) {
        found.skipped.push(format!(
            "text across column lines on page {page} is cut at them only in part: the page has \
             too many runs across its column lines"
        ));
    }
    frames
}

/// The columns of totals beside the sections `grids` (among the page's lines `hs` and `vs`,
/// those no grid takes in `taken`), with the tables of their totals planned in `tables` and
/// their runs in `claimed`.
fn totals_tables(
    page: u32,
    flow: &Flow,
    by_y: &ByY,
    (hs, vs, taken): (&[Line], &[Line], &[bool]),
    grids: &[Part],
    (claimed, tables): (&mut [Claim], &mut Vec<Planned>),
    found: &mut Found,
) -> Vec<Side> {
    let edged: Vec<(Vec<f32>, Vec<f32>)> = grids.iter().map(|p| edges(&p.h, &p.v)).collect();
    let beside: Vec<Beside<'_>> = edged
        .iter()
        .filter(|(xs, ys)| xs.len() >= 2 && ys.len() >= 2)
        .map(|(xs, ys)| Beside {
            left: xs[0],
            right: xs[xs.len() - 1],
            ys,
        })
        .collect();
    let mut work = 0;
    let sides = side_columns(flow, by_y, hs, vs, taken, &beside, &mut work);
    if work > SIDE_WORK {
        found.skipped.push(format!(
            "columns of totals beside ruled tables on page {page} are not looked for: the page \
             has too many lines"
        ));
    }
    for (side, strip) in sides
        .iter()
        .flat_map(|s| s.strips.iter().map(move |t| (s, t)))
    {
        let StripKind::Totals(totals) = &strip.kind else {
            continue;
        };
        let list: Vec<&Field> = totals.iter().collect();
        let Some(mut table) = pairs_table(
            flow,
            &list,
            totals_top(flow, totals),
            (SIDE_FOUND_BY, Detector::SideTotals),
        ) else {
            continue;
        };
        // The rule over the strip, across its column.
        table.drawn = Some([side.x0, strip.top, side.x1, strip.top]);
        claim(claimed, tables.len(), &list);
        tables.push(table);
    }
    sides
}

/// The rows of fields between rules in `frames` (of the page's horizontal rules `hs`), among
/// the runs `busy` does not name (`tabled` the runs of ruled tables), planned in `tables` with
/// their runs in `claimed`; the runs of rows left as text go to `held`.
#[allow(clippy::too_many_arguments)]
fn strip_tables(
    page: u32,
    flow: &Flow,
    by_y: &ByY,
    frames: &[Frame],
    (hs, table_rules): (&[Line], &[Line]),
    busy: &[bool],
    tabled: &[bool],
    claimed: &mut [Claim],
    held: &mut [bool],
    tables: &mut Vec<Planned>,
    found: &mut Found,
) {
    let mut strip_claims = vec![None; flow.runs.len()];
    let mut bodies = Vec::new();
    let text_sizes = layoutregions::file::text_sizes(&flow.runs);
    field_strips(
        page,
        flow,
        by_y,
        frames,
        hs,
        table_rules,
        &|i| busy[i],
        &|i| tabled[i],
        &text_sizes,
        &mut strip_claims,
        held,
        tables,
        &mut found.uncertain,
        &mut found.doubts,
        &mut bodies,
        &mut found.apart,
    );
    // The runs of each frame of column titles over items, by the frame, and its titles.
    if !bodies.is_empty() {
        found.bodies = vec![None; flow.runs.len()];
        for (ids, names) in bodies {
            let id = found.body_labels.len() as u32;
            for i in ids {
                found.bodies[i] = Some(id);
            }
            found.body_labels.push(names);
        }
    }
    for (c, s) in claimed.iter_mut().zip(strip_claims) {
        if s.is_some() {
            *c = s;
        }
    }
}

/// A section's own top rule that starts right of its left edge by more than this share of its
/// width is short of the edge: a rule a little shorter is the same rule.
const OWN_LEFT: f32 = 0.05;

/// Where the own top rule of a section at height `top` starts, when it starts right of the
/// section's left edge `left` (see [`OWN_LEFT`]; `right` is its right edge) and a rule of
/// another block at that height (among the page's `hs`, sorted by height) covers most of the
/// stretch from `left` up to it; else `left`. `h` are the section's own horizontal segments.
fn own_left(h: &[Line], hs: &[Line], top: f32, (left, right): (f32, f32)) -> f32 {
    let Some(from) = h
        .iter()
        .filter(|s| (s.at - top).abs() <= SNAP)
        .map(|s| s.from)
        .reduce(f32::min)
    else {
        return left;
    };
    if from - left <= OWN_LEFT * (right - left) {
        return left;
    }
    let lo = hs.partition_point(|s| s.at < top - SNAP);
    let other = hs[lo..]
        .iter()
        .take_while(|s| s.at <= top + SNAP)
        .any(|s| s.from <= left + SNAP && s.to < from && s.to - left >= 0.9 * (from - left));
    if other { from } else { left }
}

/// The column of totals of `sides` that stands beside the section of column edges `xs` and
/// row edges `ys` (from the top) with a strip that joins it: a strip that starts within the
/// section.
fn joined<'s>(sides: &'s [Side], xs: &[f32], ys: &[f32]) -> Option<&'s Side> {
    let (right, top, bottom) = (*xs.last()?, *ys.first()?, *ys.last()?);
    sides.iter().find(|s| {
        let gap = s.x0 - right;
        gap > SNAP
            && gap <= s.gap
            && s.strips.iter().any(|st| {
                matches!(st.kind, StripKind::Joins)
                    && st.top <= top + SNAP
                    && st.top > bottom + SNAP
            })
    })
}

/// What the title search of a section looks at and marks.
struct TitlePlace<'a> {
    flow: &'a Flow,
    by_y: &'a ByY,
    claimed: &'a [Option<(usize, usize, usize)>],
    titles: &'a mut Vec<bool>,
    /// The section's column edges.
    xs: &'a [f32],
    work: &'a mut usize,
}

impl TitlePlace<'_> {
    /// The runs between the heights `lo` and `hi` over the section that no table and no
    /// title has taken.
    fn free(&self, lo: f32, hi: f32) -> Vec<usize> {
        let (left, right) = (self.xs[0], self.xs[self.xs.len() - 1]);
        self.by_y
            .runs_between(lo, hi)
            .into_iter()
            .filter(|&i| {
                let r = &self.flow.runs[i];
                let (cx, _) = middle(r);
                !r.off_axis
                    && self.claimed[i].is_none()
                    && !self.titles[i]
                    && cx > left
                    && cx < right
                    && !r.text.trim().is_empty()
            })
            .collect()
    }

    fn take(&mut self, title: &[usize]) {
        for &i in title {
            self.titles[i] = true;
        }
    }

    /// The title right over the grid's top rule at `top`: no column rule of the page
    /// (`vs`, sorted by place) may cross it.
    fn over_grid(&mut self, vs: &[Line], top: f32) -> Vec<usize> {
        let runs = self.free(top, top + 2.0 * MAX_CAPTION_SIZE);
        let (left, right) = (self.xs[0] + SNAP, self.xs[self.xs.len() - 1] - SNAP);
        let work = &mut *self.work;
        let mut crossed = |a: f32, b: f32| column_across(vs, (left, right), (a, b), work);
        let title = title_over(&self.flow.runs, &runs, top, f32::MAX, &mut crossed);
        self.take(&title);
        title
    }

    /// The title right over the cut at the section's bottom (`ys` from the top): no inner
    /// column rule of the section (`v`) may cross it. Its runs leave the section, and a
    /// lowest row of one cell that held only them goes too, with its rules of `h`.
    fn above_cut(&mut self, v: &[Line], ys: &mut Vec<f32>, h: &mut Vec<Line>) -> Vec<usize> {
        let n = ys.len();
        let runs = self.free(ys[n - 1], ys[0]);
        let (left, right) = (self.xs[0] + SNAP, self.xs[self.xs.len() - 1] - SNAP);
        let inner = |s: &&Line| s.at > left && s.at < right;
        let low = runs
            .iter()
            .map(|&i| self.flow.runs[i].y)
            .fold(f32::MAX, f32::min);
        let ceiling = ys
            .iter()
            .copied()
            .filter(|e| *e > low)
            .fold(f32::MAX, f32::min);
        let mut crossed = |a: f32, b: f32| v.iter().filter(inner).any(|s| s.from < b && s.to > a);
        let title = title_over(&self.flow.runs, &runs, ys[n - 1], ceiling, &mut crossed);
        self.take(&title);
        if !title.is_empty() && n >= 3 {
            let (lo, hi) = (ys[n - 1], ys[n - 2]);
            let empty = runs.iter().all(|&i| {
                self.titles[i] || {
                    let (_, cy) = middle(&self.flow.runs[i]);
                    cy > hi || cy < lo
                }
            });
            let parted = v
                .iter()
                .filter(inner)
                .any(|s| s.from < hi - SNAP && s.to > lo + SNAP);
            if empty && !parted {
                ys.pop();
                h.retain(|s| s.at >= hi - SNAP);
            }
        }
        title
    }
}

/// Cuts each run that an inner column rule of a section of `parts` crosses in the white space
/// between two of its words, where the engine says (see `table::ruled::column_cuts`, with
/// the page's horizontal rules `hs`). A cut part keeps the run's place in the reading order.
/// False when more than `budget` steps were needed: the runs past them stay whole.
fn cut_at_columns(flow: &mut Flow, parts: &[Part], hs: &[Line], budget: usize) -> bool {
    let parts: Vec<GridPart<'_>> = parts
        .iter()
        .map(|p| GridPart {
            h: &p.h,
            v: &p.v,
            cut: p.cut,
        })
        .collect();
    let (cuts, done) = column_cuts(&flow.runs, &parts, hs, budget);
    apply_cuts(flow, cuts);
    done
}

/// Cuts each run `cuts` names (by its place in `flow`, in order) at its gaps listed (sorted):
/// the parts keep the run's slot and its place in the reading order.
fn apply_cuts(flow: &mut Flow, cuts: Vec<(usize, Vec<usize>)>) {
    if cuts.is_empty() {
        return;
    }
    let all = std::mem::take(flow);
    let extra: usize = cuts.iter().map(|c| c.1.len()).sum();
    let n = all.runs.len() + extra;
    let mut out = Flow {
        runs: Vec::with_capacity(n),
        slots: Vec::with_capacity(n),
        at: Vec::with_capacity(n),
        ids: Vec::with_capacity(n),
    };
    let mut next = cuts.into_iter().peekable();
    let places = all.at.into_iter().zip(all.ids);
    for (i, ((run, slot), (place, id))) in
        all.runs.into_iter().zip(all.slots).zip(places).enumerate()
    {
        let Some((_, at)) = next.next_if(|c| c.0 == i) else {
            out.runs.push(run);
            out.slots.push(slot);
            out.at.push(place);
            out.ids.push(id);
            continue;
        };
        // From the right, so the gaps to the left keep their numbers.
        let mut parts: Vec<Run> = Vec::with_capacity(at.len() + 1);
        let mut rest = run;
        for &k in at.iter().rev() {
            let Some((before, after)) = rest.cut_at(k) else {
                break;
            };
            parts.push(after);
            rest = before;
        }
        parts.push(rest);
        for part in parts.into_iter().rev() {
            out.runs.push(part);
            out.slots.push(slot);
            out.at.push(place);
            out.ids.push(id);
        }
    }
    *flow = out;
}

/// Words of a table's first cell with text that name the table in a loss.
const NAME_WORDS: usize = 6;

/// The first [`NAME_WORDS`] words of the first cell of `table` (row by row) that holds text,
/// its runs in the order of `runs`: what names the table in a loss, between quotes, so a
/// backslash in the text is escaped first (`\\`), then a quote (`\"`).
fn first_words(runs: &[Run], table: &TableOut) -> String {
    let words = |c: &layoutregions::extract::table::CellOut| {
        let mut idx = c.runs.clone();
        idx.sort_unstable();
        idx.iter()
            .flat_map(|&i| runs[i].text.split_whitespace())
            .take(NAME_WORDS)
            .collect::<Vec<_>>()
            .join(" ")
    };
    table
        .rows
        .iter()
        .flatten()
        .map(words)
        .find(|w| !w.is_empty())
        .unwrap_or_default()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// What a title drawn as an icon is written as.
const ICON: &str = "(icon)";

/// Steps one page may take to look for icons in the cells of its tables.
const ICON_WORK: usize = 1_000_000;

/// Whether drawn strokes lie inside `cell` (x0, y0, x1, y1), off its sides. `strokes` are the
/// boxes of `rules` sorted by their bottom, built at the first look.
fn drawn_inside(
    strokes: &mut Option<Vec<[f32; 4]>>,
    rules: &[Rule],
    cell: [f32; 4],
    work: &mut usize,
) -> bool {
    let boxes = strokes.get_or_insert_with(|| {
        let mut b: Vec<[f32; 4]> = rules
            .iter()
            .map(|r| {
                [
                    r.x0.min(r.x1),
                    r.y0.min(r.y1),
                    r.x0.max(r.x1),
                    r.y0.max(r.y1),
                ]
            })
            .collect();
        b.sort_by(|a, b| a[1].total_cmp(&b[1]));
        b
    });
    let lo = boxes.partition_point(|b| b[1] < cell[1] + SNAP);
    for b in &boxes[lo..] {
        *work += 1;
        if b[1] > cell[3] - SNAP || *work > ICON_WORK {
            return false;
        }
        if b[0] > cell[0] + SNAP && b[2] < cell[2] - SNAP && b[3] < cell[3] - SNAP {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(at: f32, from: f32, to: f32) -> Line {
        Line { at, from, to }
    }

    /// A run of `text` on the baseline 100 in the size 10, 6 points a character, with a gap
    /// at each stretch of spaces.
    fn spaced(text: &str, x: f32) -> Run {
        let mut gaps = Vec::new();
        let mut words = 0u32;
        let mut prev = ' ';
        for (k, c) in text.char_indices() {
            if c != ' ' && prev == ' ' && k > 0 {
                gaps.push(crate::layout::Gap {
                    words,
                    from: x + 6.0 * (text[..k].trim_end().chars().count()) as f32,
                    to: x + 6.0 * k as f32,
                });
            }
            if c != ' ' && prev == ' ' {
                words += 1;
            }
            prev = c;
        }
        Run {
            page: 1,
            text: text.into(),
            x,
            y: 100.0,
            w: 6.0 * text.chars().count() as f32,
            h: 10.0,
            size: 10.0,
            bold: false,
            italic: false,
            family: 0,
            mcid: None,
            uri: None,
            line: 0,
            off_axis: false,
            note: false,
            gaps: crate::layout::Gaps::new(gaps),
        }
    }

    /// A grid of one row from 90 to 120 over the column edges `xs`, its rules drawn the full
    /// height.
    fn one_row(xs: &[f32]) -> Part {
        let (left, right) = (xs[0], xs[xs.len() - 1]);
        Part {
            h: vec![seg(90.0, left, right), seg(120.0, left, right)],
            v: xs.iter().map(|&x| seg(x, 90.0, 120.0)).collect(),
            cut: false,
            cut_below: false,
            cut_above: false,
            id: 0,
        }
    }

    fn flow_of(runs: Vec<Run>) -> Flow {
        let n = runs.len();
        Flow::new(runs, vec![crate::layout::Slot::Free; n])
    }

    /// The engine says where a column rule cuts a run (see `table::ruled::column_cuts`); the
    /// reader cuts its own run there, and the parts keep its place.
    #[test]
    fn a_column_rule_cuts_a_run_between_two_words() {
        // "AA  BB CC" from 0: AA ends at 12, BB starts at 24; the rule at 20 lies in the gap.
        let mut flow = flow_of(vec![spaced("AA  BB CC", 0.0)]);
        assert!(cut_at_columns(
            &mut flow,
            &[one_row(&[-10.0, 20.0, 100.0])],
            &[],
            100
        ));
        let texts: Vec<&str> = flow.runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, ["AA", " BB CC"]);
        assert_eq!((flow.runs[0].x, flow.runs[0].w), (0.0, 12.0));
        assert_eq!((flow.runs[1].x, flow.runs[1].w), (24.0, 30.0));
        assert_eq!(flow.runs[1].gaps.len(), 1);
        assert_eq!(flow.runs[1].gaps[0].words, 1);
        assert_eq!(flow.at, [0, 0]);
    }

    #[test]
    fn cuts_past_the_budget_leave_the_rest_whole() {
        let runs: Vec<Run> = (0..3).map(|_| spaced("AA  BB", 0.0)).collect();
        let mut flow = flow_of(runs);
        assert!(!cut_at_columns(
            &mut flow,
            &[one_row(&[-10.0, 20.0, 100.0])],
            &[],
            2
        ));
        let texts: Vec<&str> = flow.runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, ["AA", " BB", "AA", " BB", "AA  BB"]);
    }

    /// A backslash and a quote in the first words that name a table in a loss are escaped: the
    /// name stands between quotes in the loss.
    #[test]
    fn a_quote_in_a_table_name_is_escaped() {
        use layoutregions::extract::table::CellOut;
        let runs = vec![spaced("Muster \"Plus\" A\\B GmbH", 10.0)];
        let table = TableOut {
            rows: vec![vec![
                CellOut::default(),
                CellOut {
                    runs: vec![0],
                    row_span: 1,
                    col_span: 1,
                    status: None,
                },
            ]],
            ..TableOut::default()
        };
        assert_eq!(
            first_words(&runs, &table),
            "Muster \\\"Plus\\\" A\\\\B GmbH"
        );
    }
}
