// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tables of a page, in the order the table spike found reliable: the tag tree of a tagged
//! file first, then a grid of drawn rules, then white space gutters in text without rules.
//! Each detector only claims the runs of a table and plans its cells; the layout builds the
//! cells' text, so every limit of the layout holds for tables too.

mod banded;
mod borderless;
mod pairs;
mod ruled;
mod side;
mod tagged;

use std::collections::HashMap;

use layoutregions::Bounds;
use layoutregions::segment::drawn::{self, Grids};
use layoutregions::segment::{Segment, Segments, Source};

use crate::layout::{Drawn, Flow, Run};

/// The cells of tables, built in the engine (see `layoutregions::extract::table`).
use layoutregions::extract::table::{self, MAX_SQUARES, TableOut, middle, numeric};

/// What the area of a page says against a table, and the captions next to one (see
/// `layoutregions::extract::table::{area, caption}`).
use layoutregions::extract::table::area::{
    Area, MANY_COLUMNS, MAX_COLUMNS, ON_PICTURE, PAGE_FRAME, TINY_TEXT, dense,
};
use layoutregions::extract::table::caption::{ByY, MAX_CAPTION_SIZE, caption, captions_near};

pub(crate) use pairs::{BoxTitle, Frame, box_pairs, field_frames, form_drawn, underlined_titles};
pub(crate) use tagged::{Tables, tagged};

/// A drawn line or rectangle, in points in the visible page box like the runs, `y` upward.
/// A line has `x0 == x1` or `y0 == y1`; a rectangle is its bounding box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rule {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub kind: RuleKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum RuleKind {
    Line,
    Rect,
}

/// A horizontal rule of the page, its pieces joined: its height `y`, its ends and its
/// thickness `h`. A stroked line and a filled bar of up to two points come out of the text
/// pass as lines without a thickness, so `h` is 0 for them and only a drawn rectangle has one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HRule {
    pub y: f32,
    pub x0: f32,
    pub x1: f32,
    pub h: f32,
}

/// A table found on a page, with the runs of each cell. Its cells' text is not built yet.
#[derive(Debug)]
pub(crate) struct Planned {
    /// The place of the table's first run in the page's reading order: the table stands
    /// before the runs that come after it.
    pub at: usize,
    /// The top of the table, in the frame of the runs.
    pub top: f32,
    pub rows: Vec<Vec<PlannedCell>>,
    /// For a tagged table, the row of the tag tree each of `rows` is.
    pub trs: Vec<usize>,
    /// Text for rows of a tagged table that earlier pages began.
    pub additions: Vec<Addition>,
    /// The leading rows made only of header cells.
    pub header_rows: usize,
    /// The tagged table the rows belong to, which may have started on an earlier page.
    pub tagged: Option<usize>,
    /// The rows go on a table that started on an earlier page.
    pub continues: bool,
    /// The runs of the table's title line, read as its caption.
    pub caption: Vec<Run>,
    /// The rule that found the table, when it is one the output states: a `found-by` note
    /// with this text stands before the first table of the page it found.
    pub found_by: Option<&'static str>,
    /// What the table is as a region: `LabelValue` for a table of labels and values the
    /// field reader made (drawn boxes, rows of fields, the totals of a column beside a
    /// section), else `Table`.
    pub kind: layoutregions::Kind,
    /// The ids of the runs the detector took for it (see [`Flow::ids`]), its caption and
    /// additions too, in the order taken; a run cut in parts
    /// is listed once per part.
    pub ids: Vec<u32>,
    /// The detector that found it, which names its region (see [`Detector`]).
    pub by: Detector,
    /// The box of the drawn rules or boxes the detector read the table by, in the frame of
    /// the runs; none for a detector that reads text alone. Its region reaches over it.
    pub drawn: Option<[f32; 4]>,
}

/// The detector, or the class of a detector, that found a table: its region's signal id and
/// kind (see `layoutregions`). Each detector sets it where it plans the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Detector {
    /// A table of the tag tree.
    Tagged,
    /// Drawn boxes of a label over its value.
    BoxPairs,
    /// The totals of a column beside the sections of a ruled grid.
    SideTotals,
    /// A section of a grid of drawn rules.
    Ruled,
    /// A header band of drawn cells and the lines under it.
    Banded,
    /// Rows of fields between drawn rules.
    Strips,
    /// Without rules: the totals under a table, parted from it by the rule that ends it.
    TotalsUnder,
    /// Without rules: a line of titles over one line of values.
    LabelsOverValues,
    /// Without rules: blocks side by side under a header row of their headings.
    SideBlocks,
    /// Without rules: lines of a label and its value.
    KeyValues,
    /// Without rules: any other grid of white space gutters.
    Grid,
}

impl Detector {
    /// Every detector, in the order the page runs them.
    pub(crate) const ALL: [Detector; 11] = [
        Detector::Tagged,
        Detector::BoxPairs,
        Detector::SideTotals,
        Detector::Ruled,
        Detector::Banded,
        Detector::Strips,
        Detector::TotalsUnder,
        Detector::LabelsOverValues,
        Detector::SideBlocks,
        Detector::KeyValues,
        Detector::Grid,
    ];

    /// The signal id of its regions.
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Detector::Tagged => "pdf.tagged_table",
            Detector::BoxPairs => "pdf.box_pairs",
            Detector::SideTotals => "pdf.side_totals",
            Detector::Ruled => "pdf.ruled",
            Detector::Banded => "pdf.banded",
            Detector::Strips => "pdf.strips",
            Detector::TotalsUnder => "pdf.borderless.totals_under",
            Detector::LabelsOverValues => "pdf.borderless.labels_over_values",
            Detector::SideBlocks => "pdf.borderless.side_blocks",
            Detector::KeyValues => "pdf.borderless.key_values",
            Detector::Grid => "pdf.borderless.grid",
        }
    }

    /// What a table it found of the kind `kind` (see [`Planned::kind`]) is as a region: the
    /// detectors whose tables are of one kind say it; the others keep the table's own.
    pub(crate) fn region_kind(self, kind: layoutregions::Kind) -> layoutregions::Kind {
        use layoutregions::Kind;
        match self {
            Detector::SideTotals | Detector::TotalsUnder => Kind::Totals,
            Detector::LabelsOverValues | Detector::SideBlocks | Detector::KeyValues => {
                Kind::LabelValue
            }
            // A ruled section a column of totals joins keeps its column titles over its rows:
            // a table, though the layout reads its rows as labels and values.
            Detector::Ruled | Detector::Banded | Detector::Grid => Kind::Table,
            Detector::Tagged | Detector::BoxPairs | Detector::Strips => kind,
        }
    }

    /// Whether it runs on the tables without rules: its tables come after those of the rules
    /// in the page's order of detectors, but among them by their place on the page.
    pub(crate) fn borderless(self) -> bool {
        matches!(
            self,
            Detector::TotalsUnder
                | Detector::LabelsOverValues
                | Detector::SideBlocks
                | Detector::KeyValues
                | Detector::Grid
        )
    }
}

impl Planned {
    /// Whether the table is one of labels and values that the field reader made: from drawn
    /// boxes, rows of fields or the totals of a column beside a section. Its rows are label
    /// and value, also where column titles stand over one row of items.
    pub(crate) fn is_pairs(&self) -> bool {
        self.kind == layoutregions::Kind::LabelValue
    }
}

/// Runs for a cell of a row an earlier page began: the row's place in its table and the
/// cell's place in the row.
#[derive(Debug)]
pub(crate) struct Addition {
    pub row: usize,
    pub cell: usize,
    pub runs: Vec<Run>,
}

#[derive(Debug, Default)]
pub(crate) struct PlannedCell {
    pub runs: Vec<Run>,
    pub row_span: u32,
    pub col_span: u32,
    /// What the cell says when it has no runs: a field left empty is a grey status.
    pub status: Option<&'static str>,
}

/// What a detector found on a page besides tables: a region that may be a table and is left as
/// paragraphs, with why. The layout writes it as a note where the region stands and as a loss.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Unsure {
    pub at: usize,
    pub top: f32,
    pub reason: String,
    /// The whole note, when it speaks of several regions at once.
    pub text: Option<String>,
}

/// What the detectors for untagged pages found: tables, undecided regions, and what they
/// could not look at, in words.
#[derive(Debug, Default)]
pub(crate) struct Found {
    pub tables: Vec<Planned>,
    pub unsure: Vec<Unsure>,
    pub skipped: Vec<String>,
    /// For each run of the flow, whether a grid or band refused with a note (or with a
    /// limit note for its size) holds it: no detector after it may take the run into a table.
    /// Empty when none holds a run.
    pub held: Vec<bool>,
    /// Heights of the rules of header bands, sorted: no region without rules runs across one.
    pub cuts: Vec<f32>,
    /// Grids and bands with text refused without a note: where each stands in the reading
    /// order, why and what it is. Their text is free for the detector without rules. The
    /// layout writes them as losses, one per kind and page.
    pub quiet: Vec<(usize, &'static str, Refused)>,
    /// Text a detector left as it stands, unsure what it is: the layout writes each as an
    /// Info `Uncertain` loss.
    pub uncertain: Vec<String>,
    /// For each run of the flow, whether a grid or band of `quiet` holds it: its loss speaks
    /// for the run already. Empty when none holds a run.
    pub quietly: Vec<bool>,
    /// For each run of the flow, the frame of column titles over items between rules that the
    /// field reader left to the table pass, if one holds it (see `pairs::table_body`), and the
    /// titles of each frame. Empty when none holds a run.
    pub bodies: Vec<Option<u32>>,
    pub body_labels: Vec<Vec<String>>,
    /// The horizontal rules of the page, sorted by height: a rule across a region without
    /// rules may end its table (see `borderless`). Empty when the page's lines were not
    /// searched.
    pub ends: Vec<HRule>,
    /// Rows of fields between rules that are refused or have a cell left out of their pairs:
    /// their cells, each with its runs in the order to read them (see `pairs::cell_order`). The
    /// ruled pass puts them so in the flow, each cell's lines together and apart from the next
    /// cell's.
    pub apart: Vec<Vec<Vec<usize>>>,
    /// Grids, bands, regions, rows of fields and boxes refused with a note or a loss that
    /// keep their text as it stands: the ids of their runs (see [`Flow::ids`]) and why.
    pub doubts: Doubts,
}

/// Text a detector refused with a note or a loss and keeps as it stands: for each refusal, the
/// ids of its runs (see [`Flow::ids`]) and why.
pub(crate) type Doubts = Vec<(Vec<u32>, &'static str)>;

/// What a refusal without a note refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refused {
    /// A grid of drawn rules.
    Grid,
    /// A header band and the lines below it.
    Band,
    /// A block of lines in columns without rules.
    Region,
}

/// Reasons said of several grids or bands: how many have each and why, as `2 (they cover
/// most of the page), 1 (most of its cells are empty)`, in the order the reasons first come.
pub(crate) fn counted<'a>(reasons: impl Iterator<Item = &'a str>) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for why in reasons {
        match counts.iter_mut().find(|(w, _)| *w == why) {
            Some((_, n)) => *n += 1,
            None => counts.push((why, 1)),
        }
    }
    let parts: Vec<String> = counts
        .iter()
        .map(|&(why, n)| {
            let why = if n == 1 { why } else { plural(why) };
            format!("{n} ({why})")
        })
        .collect();
    parts.join(", ")
}

/// A reason a ruled grid or a header band is refused, said of several.
pub(crate) fn plural(why: &str) -> &str {
    match why {
        MANY_COLUMNS => "they have more than 15 columns",
        "most of its cells are empty" => "most of their cells are empty",
        "its boxes do not form rows and columns" => "their boxes do not form rows and columns",
        "its text crosses its lines" => "their text crosses their lines",
        "it covers most of the page" => "they cover most of the page",
        ON_PICTURE => "they lie on a picture",
        TINY_TEXT => "their text is much smaller than the body text, as in pictures of pages",
        "too few rows follow its header" => "too few rows follow their headers",
        "the text below its header does not stand in its columns" => {
            "the text below their headers does not stand in their columns"
        }
        "its cells hold long text" => "their cells hold long text",
        "it is too tall for a header over lines" => "they are too tall for headers over lines",
        "its cells hold too few column titles" => "their cells hold too few column titles",
        "its text runs on from row to row like prose" => {
            "their text runs on from row to row like prose"
        }
        "some of its text stands between its columns" => {
            "some of their text stands between their columns"
        }
        "its columns are not clean" => "their columns are not clean",
        "all its rows read the same" => "all their rows read the same",
        "it reads like a table of contents" => "they read like tables of contents",
        other => other,
    }
}

/// Lines this close are one line, and line ends this close meet, in points; past
/// [`MAX_SEGMENTS`] straight segments the grid search gives up on a page and says so. The
/// region engine finds the grids with these values (see `layoutregions::segment::drawn`); a
/// grid or region over most of the page is its frame (see
/// `layoutregions::extract::table::area`).
pub(crate) use layoutregions::segment::drawn::{MAX_SEGMENTS, SNAP};

/// A label or a title differs in size from the text beside it when the larger is at least
/// this many times the smaller (see `layoutregions::line::LABEL_STEP`).
pub(crate) use layoutregions::line::LABEL_STEP;

/// The segments of the page the detector without rules reads, each as the union of its runs:
/// the page's root. A table often reaches past the lines drawn around part of it (a column
/// or a caption beside a grid, header cells drawn as boxes over rows that are not, labels
/// beside boxed values), so the drawn segments of a page are no bounds of its tables, and
/// neither are the whitespace and style groups under them (a bold header over its rows, a
/// blank row inside a table). Which segments a table spans is decided by typing.
pub(crate) fn table_areas(segments: &Segments) -> Vec<&Segment> {
    segments
        .all
        .get(segments.root.0 as usize)
        .into_iter()
        .collect()
}

/// For each group of `grids`, whether the ruled detector reads it: a grid of two cells or
/// more that the page's segments hold as a grid ([`Source::Grid`]), and every group of one
/// cell. A box is no grid to the segmentation, but a column of totals or a strip beside it
/// joins it as a section. Where a bound or the stop cut the segmentation short (an issue says
/// so), every group is read, as the ruled detector has its own bounds: no grid is lost to the
/// segmentation's.
fn ruled_groups(grids: &Grids, segments: &Segments) -> Vec<bool> {
    if !segments.issues.is_empty() {
        return vec![true; grids.groups.len()];
    }
    let mut frames: Vec<(Bounds, u32, u32)> = segments
        .all
        .iter()
        .filter_map(|g| match g.source {
            Source::Grid { rows, cols } => Some((g.bounds, rows, cols)),
            _ => None,
        })
        .collect();
    frames.sort_by(|a, b| a.0.x0.total_cmp(&b.0.x0));
    grids
        .groups
        .iter()
        .map(|group| {
            drawn::grid_frame(grids, group).is_none_or(|f| {
                let lo = frames.partition_point(|g| g.0.x0 < f.0.x0);
                frames[lo..]
                    .iter()
                    .take_while(|g| g.0.x0 == f.0.x0)
                    .any(|g| *g == f)
            })
        })
        .collect()
}

/// The tables of an untagged page (or a tagged page whose tag tree did not hold): first
/// grids of drawn rules, then white space gutters in the text left. Their runs are taken out
/// of `flow`. `height` is the page height; text in other directions is never in a table.
/// `segments` are the page's segments: the grids the ruled detector reads are theirs (see
/// [`ruled_groups`]), the detector without rules reads their [`table_areas`].
pub(crate) fn untagged(
    page: u32,
    flow: &mut Flow,
    drawn: &Drawn<'_>,
    segments: &Segments,
    height: f32,
    stop: &dyn Fn() -> bool,
) -> Found {
    let mut found = Found::default();
    // On a page with form fields, boxes and aligned labels are the form.
    if !drawn.widgets.is_empty() {
        return found;
    }
    let area = Area {
        width: drawn.width,
        height,
        images: drawn.images,
        body_size: drawn.body_size,
    };
    if !drawn.lines_unsearched {
        found.ends = end_rules(drawn.rules);
        let grids = drawn.grids();
        let segmented = grids.map(|g| ruled_groups(g, segments)).unwrap_or_default();
        ruled::ruled(
            page,
            flow,
            (drawn.rules, grids, &segmented),
            &area,
            stop,
            &mut found,
        );
    }
    borderless::borderless(page, flow, &table_areas(segments), &area, stop, &mut found);
    found
}

/// The horizontal rules of `rules`, sorted by height: lines with their collinear pieces
/// joined (a dashed rule is one), and rectangles wider than high as they are.
fn end_rules(rules: &[Rule]) -> Vec<HRule> {
    let mut lines = Vec::new();
    let mut out = Vec::new();
    for r in rules {
        let (x0, x1) = (r.x0.min(r.x1), r.x0.max(r.x1));
        let (y0, y1) = (r.y0.min(r.y1), r.y0.max(r.y1));
        if y1 - y0 >= x1 - x0 {
            continue;
        }
        match r.kind {
            RuleKind::Line => lines.push(layoutregions::segment::drawn::Line {
                at: (y0 + y1) / 2.0,
                from: x0,
                to: x1,
            }),
            RuleKind::Rect => out.push(HRule {
                y: (y0 + y1) / 2.0,
                x0,
                x1,
                h: y1 - y0,
            }),
        }
    }
    out.extend(
        layoutregions::segment::drawn::merged(lines)
            .into_iter()
            .map(|s| HRule {
                y: s.at,
                x0: s.from,
                x1: s.to,
                h: 0.0,
            }),
    );
    out.sort_by(|a, b| a.y.total_cmp(&b.y));
    out
}

/// One line of the flow: the range of its runs and its baseline and size (see
/// [`table::RunRange`]).
type FlowLine = table::RunRange;

/// The lines of the flow (see [`table::run_ranges`]).
fn flow_lines(flow: &Flow) -> Vec<FlowLine> {
    table::run_ranges(&flow.runs)
}

/// Where a run of the page goes: the table, row and cell of `planned`.
type Target = Option<(u32, u32, u32)>;

/// The row of a [`Target`] that sends a run to the table's caption.
const CAPTION_ROW: u32 = u32::MAX;

/// Moves the runs `target` names out of `flow` into the cells of `planned`. Each cell is sized
/// for its runs first: a list grown one run at a time keeps room for four.
fn fill(planned: &mut [Planned], flow: &mut Flow, target: &[Target]) {
    let mut counts: HashMap<(u32, u32, u32), usize> = HashMap::new();
    for t in target.iter().flatten() {
        *counts.entry(*t).or_default() += 1;
    }
    for ((t, r, c), n) in counts {
        if r == CAPTION_ROW {
            continue;
        }
        planned[t as usize].rows[r as usize][c as usize]
            .runs
            .reserve_exact(n);
    }
    for (t, id) in target.iter().zip(&flow.ids) {
        if let Some((t, _, _)) = t {
            planned[*t as usize].ids.push(*id);
        }
    }
    let stay = target.iter().filter(|t| t.is_none()).count()
        + flow.runs.len().saturating_sub(target.len());
    flow.take_each(stay, |i, r| match target.get(i).copied().flatten() {
        Some((t, CAPTION_ROW, _)) => {
            planned[t as usize].caption.push(r);
            None
        }
        Some((t, row, cell)) => {
            planned[t as usize].rows[row as usize][cell as usize]
                .runs
                .push(r);
            None
        }
        None => Some(r),
    });
}

/// Tables without tags one page may have; past them the text stays paragraphs, with a loss.
const MAX_TABLES: usize = 5_000;

/// The loss for a page that has more tables than it may.
fn too_many_tables(page: u32) -> String {
    format!(
        "tables on page {page} past the first {MAX_TABLES} are not rebuilt: the page has more \
         tables than it may; their text stays as paragraphs"
    )
}

/// Grids and regions one page may have looked at between two looks at the clock.
const CHECK_EVERY: usize = 64;

/// Claims the runs of the cells of `table`, the table `t` of the page, in `claimed`.
fn claim_cells(claimed: &mut [Option<(usize, usize, usize)>], t: usize, table: &TableOut) {
    for (r, row) in table.rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            for &i in &cell.runs {
                claimed[i] = Some((t, r, c));
            }
        }
    }
}

/// The rows of `table` for a planned table: each cell with its spans and status, its runs
/// filled in once they are taken out of the flow.
fn planned_rows(table: &TableOut) -> Vec<Vec<PlannedCell>> {
    table
        .rows
        .iter()
        .map(|r| {
            r.iter()
                .map(|c| PlannedCell {
                    runs: Vec::new(),
                    row_span: c.row_span,
                    col_span: c.col_span,
                    status: c.status,
                })
                .collect()
        })
        .collect()
}

/// 1 when the first row reads as column titles (see [`table::header_of`]).
fn header_of(rows: &[Vec<PlannedCell>]) -> usize {
    let view: Vec<Vec<&[Run]>> = rows
        .iter()
        .map(|r| r.iter().map(|c| c.runs.as_slice()).collect())
        .collect();
    table::header_of(&view)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planned(kind: layoutregions::Kind, found_by: Option<&'static str>) -> Planned {
        Planned {
            at: 0,
            top: 0.0,
            rows: Vec::new(),
            trs: Vec::new(),
            additions: Vec::new(),
            header_rows: 0,
            tagged: None,
            continues: false,
            caption: Vec::new(),
            found_by,
            kind,
            ids: Vec::new(),
            by: Detector::Grid,
            drawn: None,
        }
    }

    /// A detector of one kind of table gives its regions that kind; the others keep the
    /// table's own. A ruled section a column of totals joins is a table region, while the
    /// layout still reads it as labels and values.
    #[test]
    fn region_kinds_by_detector() {
        use layoutregions::Kind;
        assert_eq!(
            Detector::SideTotals.region_kind(Kind::LabelValue),
            Kind::Totals
        );
        assert_eq!(Detector::TotalsUnder.region_kind(Kind::Table), Kind::Totals);
        assert_eq!(
            Detector::KeyValues.region_kind(Kind::Table),
            Kind::LabelValue
        );
        assert_eq!(Detector::Ruled.region_kind(Kind::LabelValue), Kind::Table);
        assert_eq!(
            Detector::Strips.region_kind(Kind::LabelValue),
            Kind::LabelValue
        );
        assert_eq!(
            Detector::BoxPairs.region_kind(Kind::LabelValue),
            Kind::LabelValue
        );
        let ids: std::collections::HashSet<&str> = Detector::ALL.iter().map(|d| d.id()).collect();
        assert_eq!(ids.len(), Detector::ALL.len());
    }

    /// Whether a table is one of labels and values is its kind, not the text of its note.
    #[test]
    fn is_pairs_comes_from_the_kind() {
        assert!(planned(layoutregions::Kind::LabelValue, None).is_pairs());
        assert!(!planned(layoutregions::Kind::Table, None).is_pairs());
        assert!(!planned(layoutregions::Kind::Table, Some(side::SIDE_FOUND_BY)).is_pairs());
    }

    /// A page with a single drawn box at x 50 and two grids of 2 x 2 cells at x 250 and 400,
    /// a word in each cell.
    fn box_and_grids() -> layoutregions::PageIn {
        let rule = |x0: f32, y0: f32, x1: f32, y1: f32, rect: bool| layoutregions::RuleIn {
            x0,
            y0,
            x1,
            y1,
            thick: 0.5,
            dashed: false,
            rect,
        };
        let mut rules = vec![rule(50.0, 600.0, 150.0, 620.0, true)];
        let mut texts = vec![(60.0, 606.0)];
        for x in [250.0, 400.0] {
            rules.push(rule(x, 600.0, x + 100.0, 640.0, true));
            rules.push(rule(x + 50.0, 600.0, x + 50.0, 640.0, false));
            rules.push(rule(x, 620.0, x + 100.0, 620.0, false));
            for (dx, dy) in [(5.0, 606.0), (55.0, 606.0), (5.0, 626.0), (55.0, 626.0)] {
                texts.push((x + dx, dy));
            }
        }
        layoutregions::PageIn {
            page: 1,
            size: [595.0, 842.0],
            turn: 0,
            texts: texts
                .iter()
                .enumerate()
                .map(|(k, &(x, y))| layoutregions::TextRun {
                    text: "Muster".into(),
                    x,
                    y,
                    w: 30.0,
                    h: 8.0,
                    size: 8.0,
                    font: 1,
                    font_kind: layoutregions::FontKind::Proportional,
                    bold: false,
                    italic: false,
                    color: None,
                    shift: layoutregions::Shift::Base,
                    line: k as u32,
                    off_axis: false,
                    gaps: Vec::new(),
                })
                .collect(),
            rules,
            boxes: Vec::new(),
            pictures: Vec::new(),
            codes: Vec::new(),
            given: Vec::new(),
        }
    }

    #[test]
    fn ruled_groups_follow_the_grid_segments() {
        let page = box_and_grids();
        let grids = drawn::grids(&page.rules).expect("searched");
        assert_eq!(grids.groups.len(), 3);
        let mut segments =
            layoutregions::segment::segment_page(&page, &layoutregions::Limits::default(), &|| {
                false
            });
        assert!(segments.issues.is_empty());
        // The x of each group's left edge.
        let lefts: Vec<f32> = grids
            .groups
            .iter()
            .map(|(gh, _)| {
                gh.iter()
                    .map(|&j| grids.hs[j].from)
                    .fold(f32::MAX, f32::min)
            })
            .collect();
        let want = |box_and_first: bool, second: bool| -> Vec<bool> {
            lefts
                .iter()
                .map(|&x| if x > 390.0 { second } else { box_and_first })
                .collect()
        };
        // The box is read as a section of its own; both grids have their segments.
        assert_eq!(ruled_groups(&grids, &segments), want(true, true));
        // A grid the segments do not hold is not read.
        for g in &mut segments.all {
            if matches!(g.source, Source::Grid { .. }) && g.bounds.x0 > 390.0 {
                g.source = Source::Rest;
            }
        }
        assert_eq!(ruled_groups(&grids, &segments), want(true, false));
        // Where a bound or the stop cut the segmentation short, every group is read.
        for kind in [
            layoutregions::IssueKind::Limit,
            layoutregions::IssueKind::Stopped,
        ] {
            let mut cut = segments.clone();
            cut.issues.push(layoutregions::Issue {
                what: "the segmentation was cut short".into(),
                kind,
            });
            assert_eq!(ruled_groups(&grids, &cut), want(true, true), "{kind:?}");
        }
    }
}
