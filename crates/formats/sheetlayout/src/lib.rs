// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Finds the tables, titles and blocks inside one spreadsheet sheet.
//!
//! The input is the sheet's sparse cell list with style flags, its merges, the layout facts
//! the file states (Excel tables, database ranges, pivots, autofilters), defined names, panes
//! and print titles. The output is a list of regions in reading order, each with its exact
//! range, how it was found and how sure that is. Facts decide first; rules decide only where
//! no fact exists.
//!
//! The crate never builds a grid of the used area. It works on the sorted cells and on
//! rectangles, so a merge over the whole sheet costs as much as one cell.
//!
//! ```
//! use sheetlayout::{CellIn, Class, Input, Limits, Pos, Style, segment};
//!
//! let cell = |row, col, text: &str| CellIn {
//!     pos: Pos { row, col },
//!     class: Class::Text,
//!     text: text.to_string(),
//!     number: None,
//!     style: Style::default(),
//!     formula: None,
//! };
//! let input = Input {
//!     cells: vec![cell(0, 0, "a"), cell(0, 1, "b"), cell(4, 0, "c")],
//!     merges: Vec::new(),
//!     facts: Vec::new(),
//!     names: Vec::new(),
//!     outline: Vec::new(),
//!     frozen: None,
//!     print_titles: None,
//!     header_guess: None,
//!     styles_read: false,
//!     is_label: &|_| true,
//!     is_unit_column: &|_| false,
//! };
//! let layout = segment(&input, &Limits::default(), &|| false);
//! let ranges: Vec<String> = layout.regions.iter().map(|r| r.rect.a1()).collect();
//! assert_eq!(ranges, ["A1:B1", "A5"]);
//! ```

mod captions;
mod cut;
mod facts;
mod input;
mod occupancy;

pub use input::{
    CellIn, Class, Fact, FactKind, FormulaIn, Input, Limits, NameIn, Pos, Rect, Style,
    column_letters,
};

use cut::Trip;
use occupancy::{Prep, bounds};

/// The kind of a region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Table,
    KeyValue,
    Text,
    /// Caption rows only, written as a heading and lines.
    Title,
    /// The raw cells, when the rules are unsure.
    Grid,
}

/// How sure a region or its header is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Certainty {
    /// Decided by a fact in the file.
    Exact,
    /// Decided by rules that agree.
    Rule,
    /// The rules disagree, or none applies.
    Unsure,
}

/// What decided a region or its header.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Evidence {
    ListObject {
        name: String,
    },
    DatabaseRange {
        name: String,
    },
    Pivot {
        name: String,
    },
    AutoFilter,
    PrintTitles,
    FrozenPanes,
    /// A defined name that joins the region or names its exact range. When the region has no
    /// caption rows, a name that is not hidden stands as its caption.
    DefinedName {
        name: String,
    },
    /// A rule: `G`, `T`, `T2`, `N`, `S`, `C`, `H0`, `H1`, `H2`, `H3`, `L`, `R1`, `R2`, `R3`,
    /// `K` or `X`, with the words of the layout line.
    Rule {
        id: &'static str,
        detail: String,
    },
}

/// The kind of a table row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowKind {
    Data,
    Subtotal,
    Total,
    Group,
    Blank,
}

/// A row with a kind or a level.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RowNote {
    pub row: u32,
    pub kind: RowKind,
    pub level: Option<u8>,
    pub parent: Option<u32>,
    pub why: String,
}

/// One region of a sheet. Rows and columns are 0-based sheet positions.
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    pub kind: Kind,
    /// The whole region, with caption and note rows.
    pub rect: Rect,
    /// The data area: the rows after the caption and header rows.
    pub data: Option<Rect>,
    /// Caption rows above the region.
    pub captions: Vec<u32>,
    /// The caption row written as the heading.
    pub caption_heading: Option<u32>,
    pub notes: Vec<u32>,
    /// A section heading row and its depth.
    pub heading: Option<(u32, u8)>,
    pub header_rows: Vec<u32>,
    pub header_paths: Vec<String>,
    pub label_cols: u32,
    pub certainty: Certainty,
    pub evidence: Vec<Evidence>,
    /// Why the region is unsure, in words.
    pub unsure: Vec<String>,
    pub rows: Vec<RowNote>,
    /// The file states the table has no header row in the sheet.
    pub hidden_header: bool,
}

/// What an issue is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IssueKind {
    /// A work limit was reached; the rest of the sheet is one grid region.
    Limit,
    /// Two facts of the same level overlap.
    Overlap,
    /// The stop callback ended the work; the rest of the sheet is one grid region.
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub what: String,
    pub kind: IssueKind,
    pub rect: Option<Rect>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// In reading order: top row, then left column.
    pub regions: Vec<Region>,
    pub issues: Vec<Issue>,
}

/// Where a region came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
    Fact,
    Rule,
    /// The rest of the sheet after a limit or a stop.
    Rest,
}

/// A region while it is being worked on, with the merges it holds.
pub(crate) struct Work {
    pub region: Region,
    /// Indices into `Prep::merges`.
    pub merges: Vec<u32>,
    pub origin: Origin,
    pub dead: bool,
}

fn rule_region(rect: Rect, detail: &str) -> Region {
    Region {
        kind: Kind::Grid,
        rect,
        data: Some(rect),
        captions: Vec::new(),
        caption_heading: None,
        notes: Vec::new(),
        heading: None,
        header_rows: Vec::new(),
        header_paths: Vec::new(),
        label_cols: 0,
        certainty: Certainty::Unsure,
        evidence: vec![Evidence::Rule {
            id: "G",
            detail: detail.to_string(),
        }],
        unsure: vec!["not decided".to_string()],
        rows: Vec::new(),
        hidden_header: false,
    }
}

fn gap_words(leaf: &cut::Leaf) -> String {
    let gaps = match (leaf.rows_cut, leaf.cols_cut) {
        (true, true) => "empty-row and empty-column gaps",
        (true, false) => "empty-row gaps",
        (false, true) => "empty-column gaps",
        (false, false) => "no gaps",
    };
    let mut words = gaps.to_string();
    if leaf.beside {
        words.push_str(", cut around a table range");
    }
    if leaf.corner {
        words.push_str(", apart from a block it touches only at a corner");
    }
    words
}

/// The items not placed become one grid region. Every region its rectangle reaches is folded
/// in, so no cell is in two regions.
fn rest_region(
    prep: &Prep,
    works: &mut [Work],
    ids: &[u32],
    why: Trip,
    limits: &Limits,
    issues: &mut Vec<Issue>,
) -> Option<Work> {
    let mut rect = bounds(prep, ids)?;
    let mut merges: Vec<u32> = ids.iter().filter_map(|&id| prep.merge_of(id)).collect();
    loop {
        let mut grew = false;
        for w in works.iter_mut() {
            if !w.dead && w.region.rect.intersects(&rect) {
                rect = rect.union(&w.region.rect);
                merges.append(&mut w.merges);
                w.dead = true;
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    let (what, reason, kind) = match why {
        Trip::Regions => (
            format!("region limit of {} reached", limits.max_regions),
            "region limit",
            IssueKind::Limit,
        ),
        Trip::Depth => (
            format!("cut depth limit of {} reached", limits.max_cut_depth),
            "cut depth limit",
            IssueKind::Limit,
        ),
        Trip::Stopped => (
            "time limit reached".to_string(),
            "time limit",
            IssueKind::Stopped,
        ),
    };
    issues.push(Issue {
        what: format!(
            "{what}: the rest of the sheet, {}, is one grid region",
            rect.a1()
        ),
        kind,
        rect: Some(rect),
    });
    let mut region = rule_region(rect, &format!("rest of the sheet after the {reason}"));
    region.unsure = vec![what];
    Some(Work {
        region,
        merges,
        origin: Origin::Rest,
        dead: false,
    })
}

/// Steps 1 to 5 of the detection order, with the merges each region holds.
pub(crate) fn segment_work<'a>(
    input: &'a Input<'a>,
    limits: &Limits,
    stop: &dyn Fn() -> bool,
) -> (Prep<'a>, Vec<Work>, Vec<Issue>) {
    let prep = Prep::new(input);
    let max = limits.max_regions.max(1);
    let mut issues = Vec::new();

    // 1. Fact regions.
    let kept = facts::resolve(input, max - 1, &mut issues);
    let held = facts::assign(&prep, &kept, stop);
    let mut works: Vec<Work> = kept
        .iter()
        .zip(held.merges)
        .map(|(k, merges)| Work {
            region: facts::region(input, k),
            merges,
            origin: Origin::Fact,
            dead: false,
        })
        .collect();

    // 2. Rule G on the items no fact holds.
    let free: Vec<u32> = (0..prep.items() as u32)
        .filter(|&id| !held.taken[id as usize])
        .collect();
    let rest = if held.stopped {
        (!free.is_empty()).then_some((Trip::Stopped, free))
    } else {
        let room = max.saturating_sub(works.len()).max(1);
        let fact_rects: Vec<Rect> = kept.iter().map(|k| k.rect).collect();
        let cut = cut::cut(&prep, free, &fact_rects, limits, room, stop);
        for leaf in cut.leaves {
            let rect = cut::leaf_rect(&prep, &leaf);
            works.push(Work {
                region: rule_region(rect, &gap_words(&leaf)),
                merges: leaf
                    .ids
                    .iter()
                    .filter_map(|&id| prep.merge_of(id))
                    .collect(),
                origin: Origin::Rule,
                dead: false,
            });
        }
        cut.rest
    };
    if let Some((why, ids)) = rest
        && let Some(w) = rest_region(&prep, &mut works, &ids, why, limits, &mut issues)
    {
        works.push(w);
    }

    // 3. Joining by defined names.
    facts::join_names(input, &mut works, max, &mut issues);

    // 4. Caption rows, then title regions.
    for w in works.iter_mut() {
        if !w.dead && w.origin == Origin::Rule {
            captions::caption_rows(&prep, w);
        }
    }
    captions::attach_titles(&mut works);

    // 5. Print titles and frozen panes, kept as evidence for the header rules.
    for w in works.iter_mut() {
        if w.dead || w.origin == Origin::Rest {
            continue;
        }
        let r = w.region.rect;
        if let Some((a, b)) = input.print_titles {
            let (a, b) = (a.min(b), a.max(b));
            if r.top <= a && b <= r.bottom {
                w.region.evidence.push(Evidence::PrintTitles);
            }
        }
        if let Some((rows, cols)) = input.frozen {
            let by_rows = rows > 0 && r.top < rows && rows <= r.bottom;
            let by_cols = cols > 0 && r.left < cols && cols <= r.right;
            if by_rows || by_cols {
                w.region.evidence.push(Evidence::FrozenPanes);
            }
        }
    }

    works.retain(|w| !w.dead);
    works.sort_by_key(|w| (w.region.rect.top, w.region.rect.left));
    (prep, works, issues)
}

/// Finds the regions of one sheet: fact regions, rule G regions, joins by defined names,
/// caption rows and title regions, and the print titles and panes that cover each region.
///
/// Fact regions are tables with an exact header. Rule regions are grids with certainty
/// `Unsure` until the header rules decide them. `stop` is asked between pieces of work; when
/// it says stop, the cells not yet placed become one grid region and a `Stopped` issue says
/// so. Past a limit the same happens with a `Limit` issue.
pub fn segment(input: &Input, limits: &Limits, stop: &dyn Fn() -> bool) -> Layout {
    let (_, works, issues) = segment_work(input, limits, stop);
    Layout {
        regions: works.into_iter().map(|w| w.region).collect(),
        issues,
    }
}
