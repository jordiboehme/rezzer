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
//! [`segment`] finds the regions; [`detect`] also decides the header band, the header paths
//! and the label columns of each region.
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
mod detect;
mod facts;
mod header;
mod input;
mod occupancy;
mod paths;

pub use input::{
    CellIn, Class, Fact, FactKind, FormulaIn, Input, Limits, NameIn, Pos, Rect, Style,
    column_letters,
};

pub use detect::detect;

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
    /// Rows of a fact above its header band, such as a pivot's data caption row. Each is
    /// written as one line with its cells joined.
    pub above_header: Vec<u32>,
    pub header_rows: Vec<u32>,
    /// The table body: a fact's own range, or the data area of a rule region (the rows after
    /// its captions, in the table's own columns). Set for every region with header paths. A
    /// title attached above can make `rect` wider than the body.
    pub body: Option<Rect>,
    /// One path per column of `body`: `header_paths[i]` is column `body.left + i`. An empty
    /// path means no band cell names the column; the caller writes the column letter.
    pub header_paths: Vec<String>,
    /// The left columns of `body` that name the rows, counted from `body.left`.
    pub label_cols: u32,
    pub certainty: Certainty,
    pub evidence: Vec<Evidence>,
    /// Why the region is unsure, in words.
    pub unsure: Vec<String>,
    pub rows: Vec<RowNote>,
    /// The file states the table has no header row in the sheet.
    pub hidden_header: bool,
    /// The rest of the sheet after a limit or a stop: one grid of the cells not placed, which
    /// no rule decides. A region whose header paths would pass the path limit is one too. An
    /// issue with the same rectangle says why.
    pub rest: bool,
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
    /// Information: the region may hold more than one block, because the corner split needed
    /// more work than its budget. The region is decided by the rules like any other. A caller
    /// maps it to an informational note.
    Coarse,
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
    /// The corner split was skipped for this region (it may hold several blocks).
    pub coarse: bool,
    /// For a fact region, its index into `Input::facts`.
    pub fact: Option<usize>,
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
        above_header: Vec::new(),
        header_rows: Vec::new(),
        body: None,
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
        rest: false,
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
    loose: &[u32],
    why: Trip,
    limits: &Limits,
    issues: &mut Vec<Issue>,
) -> Option<Work> {
    let mut rect = bounds(prep, ids)?;
    let mut merges: Vec<u32> = ids.iter().filter_map(|&id| prep.merge_of(id)).collect();
    merges.extend_from_slice(loose);
    loop {
        let mut grew = false;
        for w in works.iter_mut() {
            if !w.dead && w.region.rect.intersects(&rect) {
                if w.region.rest {
                    // A piece past the depth limit is folded in: its issue names a range that
                    // no longer has a region of its own.
                    let gone = Some(w.region.rect);
                    issues.retain(|i| !(i.kind == IssueKind::Limit && i.rect == gone));
                }
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
    region.rest = true;
    Some(Work {
        region,
        merges,
        origin: Origin::Rest,
        dead: false,
        coarse: false,
        fact: None,
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
    let (kept, stopped) = facts::resolve(input, max - 1, &mut issues, stop);
    let mut held = facts::assign(&prep, &kept, &|| stopped || stop());
    held.stopped |= stopped;
    let mut works: Vec<Work> = kept
        .iter()
        .zip(held.merges)
        .map(|(k, merges)| Work {
            region: facts::region(input, k),
            merges,
            origin: Origin::Fact,
            dead: false,
            coarse: false,
            fact: Some(k.fact),
        })
        .collect();

    // 2. Rule G on the items no fact holds.
    let free: Vec<u32> = (0..prep.items() as u32)
        .filter(|&id| !held.taken[id as usize])
        .collect();
    let rest = if held.stopped {
        (!free.is_empty()).then_some((Trip::Stopped, free, Vec::new()))
    } else {
        let room = max.saturating_sub(works.len()).max(1);
        let fact_rects: Vec<Rect> = kept.iter().map(|k| k.rect).collect();
        let cut = cut::cut(&prep, free, &fact_rects, limits, room, stop);
        for leaf in cut.leaves {
            let rect = cut::leaf_rect(&prep, &leaf);
            let mut merges: Vec<u32> = leaf
                .ids
                .iter()
                .filter_map(|&id| prep.merge_of(id))
                .collect();
            merges.extend_from_slice(&leaf.loose);
            let (region, origin) = if leaf.deep {
                let what = format!("cut depth limit of {} reached", limits.max_cut_depth);
                issues.push(Issue {
                    what: format!("{what}: {} is one grid region", rect.a1()),
                    kind: IssueKind::Limit,
                    rect: Some(rect),
                });
                let mut region = rule_region(rect, "a piece past the cut depth limit");
                region.unsure = vec![what];
                region.rest = true;
                (region, Origin::Rest)
            } else {
                (rule_region(rect, &gap_words(&leaf)), Origin::Rule)
            };
            works.push(Work {
                region,
                merges,
                origin,
                dead: false,
                coarse: leaf.coarse,
                fact: None,
            });
        }
        cut.rest
    };
    if let Some((why, ids, loose)) = rest
        && let Some(w) = rest_region(&prep, &mut works, &ids, &loose, why, limits, &mut issues)
    {
        works.push(w);
    }

    // 3. Joining by defined names.
    facts::join_names(input, &mut works, max, &mut issues, stop);

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

    for w in works.iter().filter(|w| !w.dead && w.coarse) {
        issues.push(Issue {
            what: format!(
                "the corner split needs too much work: {} stays one region and may hold more than one block",
                w.region.rect.a1()
            ),
            kind: IssueKind::Coarse,
            rect: Some(w.region.rect),
        });
    }
    works.retain(|w| !w.dead);
    works.sort_by_key(|w| (w.region.rect.top, w.region.rect.left));
    (prep, works, issues)
}

/// Finds the regions of one sheet: fact regions, rule G regions, joins by defined names,
/// caption rows and title regions, and the print titles and panes that cover each region.
///
/// Fact regions are tables with an exact header. Rule regions are grids with certainty
/// `Unsure` and the reason `not decided`; [`detect`] decides them. `stop` is asked between pieces of work; when
/// it says stop, the cells not yet placed become one grid region and a `Stopped` issue says
/// so. Past the region limit the same happens with a `Limit` issue; a piece past the cut
/// depth limit stays one grid region of its own with a `Limit` issue. Such rest regions have
/// `rest` set, and their issue carries the same rectangle.
pub fn segment(input: &Input, limits: &Limits, stop: &dyn Fn() -> bool) -> Layout {
    let (_, works, issues) = segment_work(input, limits, stop);
    Layout {
        regions: works.into_iter().map(|w| w.region).collect(),
        issues,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(row: u32, col: u32) -> CellIn {
        CellIn {
            pos: Pos { row, col },
            class: Class::Text,
            text: "t".to_string(),
            number: None,
            style: Style::default(),
            formula: None,
        }
    }

    fn input(cells: Vec<CellIn>, merges: Vec<Rect>, fact: Rect) -> Input<'static> {
        input_with(cells, merges, vec![fact])
    }

    fn input_with(mut cells: Vec<CellIn>, merges: Vec<Rect>, facts: Vec<Rect>) -> Input<'static> {
        cells.sort_by_key(|c| c.pos);
        Input {
            cells,
            merges,
            facts: facts
                .into_iter()
                .map(|rect| Fact {
                    rect,
                    kind: FactKind::AutoFilter,
                })
                .collect(),
            names: Vec::new(),
            outline: Vec::new(),
            frozen: None,
            print_titles: None,
            header_guess: None,
            styles_read: false,
            is_label: &|_| true,
            is_unit_column: &|_| false,
        }
    }

    fn rect(top: u32, left: u32, bottom: u32, right: u32) -> Rect {
        Rect {
            top,
            left,
            bottom,
            right,
        }
    }

    /// Every merge is listed by exactly one region.
    fn merges_once(input: &Input) {
        let (prep, works, _) = segment_work(input, &Limits::default(), &|| false);
        for m in 0..prep.merges.len() as u32 {
            let n = works.iter().filter(|w| w.merges.contains(&m)).count();
            assert_eq!(
                n, 1,
                "merge {:?} is in {n} regions",
                prep.merges[m as usize]
            );
        }
    }

    #[test]
    fn merge_reaching_into_a_fact_belongs_to_the_fact() {
        let mut cells = vec![text(0, 0)];
        for row in 2..=4 {
            for col in 0..=2 {
                cells.push(text(row, col));
            }
        }
        let input = input(cells, vec![rect(0, 0, 4, 0)], rect(2, 0, 4, 2));
        merges_once(&input);
        let (_, works, _) = segment_work(&input, &Limits::default(), &|| false);
        let fact = works.iter().find(|w| w.origin == Origin::Fact).unwrap();
        assert_eq!(fact.merges, [0]);
    }

    #[test]
    fn merge_crossing_a_line_around_a_fact_stays_with_its_anchor() {
        // A1 merged over A1:A3 beside a fact B2:D5, with a column E and row 6 around it.
        let mut cells = vec![text(0, 0)];
        for row in 0..=5 {
            cells.push(text(row, 4));
        }
        for col in 0..=3 {
            cells.push(text(5, col));
        }
        for row in 1..=4 {
            for col in 1..=3 {
                cells.push(text(row, col));
            }
        }
        let input = input(cells, vec![rect(0, 0, 2, 0)], rect(1, 1, 4, 3));
        merges_once(&input);
        let (_, works, _) = segment_work(&input, &Limits::default(), &|| false);
        let holder = works.iter().find(|w| w.merges == [0]).unwrap();
        assert!(holder.region.rect.contains(Pos { row: 0, col: 0 }));
    }

    #[test]
    fn merge_reaching_two_facts_belongs_to_the_one_with_its_anchor() {
        // The first fact (C3:E5) is reached; the second (A1:B2) holds the anchor A1.
        let mut cells = Vec::new();
        for row in 0..=1 {
            for col in 0..=1 {
                cells.push(text(row, col));
            }
        }
        for row in 2..=4 {
            for col in 2..=4 {
                cells.push(text(row, col));
            }
        }
        let input = input_with(
            cells,
            vec![rect(0, 0, 3, 2)],
            vec![rect(2, 2, 4, 4), rect(0, 0, 1, 1)],
        );
        merges_once(&input);
        let (_, works, _) = segment_work(&input, &Limits::default(), &|| false);
        let holder = works.iter().find(|w| w.merges == [0]).unwrap();
        assert_eq!(holder.region.rect, rect(0, 0, 1, 1));
    }

    #[test]
    fn loose_merge_goes_to_the_part_with_its_anchor() {
        // A fact B2:D5 ringed by row 1, column A, column E and row 6. The merge A3:A6 crosses
        // the line below the fact; its anchor A3 is in the part left of the fact, while a part
        // above it exists too.
        let mut cells = Vec::new();
        for col in 0..=4 {
            cells.push(text(0, col));
            cells.push(text(5, col));
        }
        for row in 1..=4 {
            cells.push(text(row, 0));
            cells.push(text(row, 4));
            for col in 1..=3 {
                cells.push(text(row, col));
            }
        }
        let input = input(cells, vec![rect(2, 0, 5, 0)], rect(1, 1, 4, 3));
        merges_once(&input);
        let (_, works, _) = segment_work(&input, &Limits::default(), &|| false);
        let holder = works.iter().find(|w| w.merges == [0]).unwrap();
        assert_eq!(holder.region.rect, rect(1, 0, 4, 0));
    }
}
