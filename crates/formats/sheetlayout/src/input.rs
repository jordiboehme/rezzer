// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the caller gives: cells, merges, layout facts, names and signals of one sheet.

/// A 0-based sheet position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    pub row: u32,
    pub col: u32,
}

/// An inclusive 0-based rectangle of sheet positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rect {
    pub top: u32,
    pub left: u32,
    pub bottom: u32,
    pub right: u32,
}

/// The column letters of a 0-based column: 0 is `A`, 26 is `AA`.
pub fn column_letters(col: u32) -> String {
    let mut n = u64::from(col);
    let mut letters = Vec::new();
    loop {
        letters.push(char::from(b'A' + (n % 26) as u8));
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    letters.iter().rev().collect()
}

impl Rect {
    /// The one-cell rectangle at `p`.
    pub fn cell(p: Pos) -> Self {
        Self {
            top: p.row,
            left: p.col,
            bottom: p.row,
            right: p.col,
        }
    }

    /// The same rectangle with its corners in order: an inverted rectangle from a damaged file
    /// becomes the one its corners span.
    pub fn normalized(self) -> Self {
        Self {
            top: self.top.min(self.bottom),
            left: self.left.min(self.right),
            bottom: self.top.max(self.bottom),
            right: self.left.max(self.right),
        }
    }

    /// The A1 address: `B3:D5`, or `B3` for one cell.
    pub fn a1(&self) -> String {
        let corner = |row: u32, col: u32| format!("{}{}", column_letters(col), u64::from(row) + 1);
        let first = corner(self.top, self.left);
        if self.top == self.bottom && self.left == self.right {
            first
        } else {
            format!("{first}:{}", corner(self.bottom, self.right))
        }
    }

    pub fn contains(&self, p: Pos) -> bool {
        self.top <= p.row && p.row <= self.bottom && self.left <= p.col && p.col <= self.right
    }

    /// Whether `other` lies wholly inside this rectangle.
    pub fn covers(&self, other: &Rect) -> bool {
        self.top <= other.top
            && other.bottom <= self.bottom
            && self.left <= other.left
            && other.right <= self.right
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.top <= other.bottom
            && other.top <= self.bottom
            && self.left <= other.right
            && other.left <= self.right
    }

    /// The common part, when there is one.
    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        self.intersects(other).then(|| Rect {
            top: self.top.max(other.top),
            left: self.left.max(other.left),
            bottom: self.bottom.min(other.bottom),
            right: self.right.min(other.right),
        })
    }

    /// The smallest rectangle that holds both.
    pub fn union(&self, other: &Rect) -> Rect {
        Rect {
            top: self.top.min(other.top),
            left: self.left.min(other.left),
            bottom: self.bottom.max(other.bottom),
            right: self.right.max(other.right),
        }
    }

    /// The number of rows, as u64 so a whole-sheet rectangle cannot overflow.
    pub fn height(&self) -> u64 {
        u64::from(self.bottom) - u64::from(self.top) + 1
    }

    pub fn width(&self) -> u64 {
        u64::from(self.right) - u64::from(self.left) + 1
    }
}

/// The type class of a cell value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    Text,
    Number,
    Date,
    Bool,
    Error,
}

/// Style signals of a cell. `None` is unknown (the format does not give it or it was not
/// read), never false.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Style {
    pub bold: Option<bool>,
    pub filled: Option<bool>,
    pub indent: Option<u8>,
    pub center_across: Option<bool>,
    pub rule_above: Option<bool>,
    pub rule_below: Option<bool>,
}

/// What a formula cell calls and reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormulaIn {
    /// Function names, upper case.
    pub functions: Vec<String>,
    /// Ranges on the same sheet that the formula reads.
    pub reads: Vec<Rect>,
}

/// One cell with content. A formula cell with an empty result has an empty `text` and still
/// counts as occupied; any other cell with an empty `text` counts as empty.
#[derive(Debug, Clone, PartialEq)]
pub struct CellIn {
    pub pos: Pos,
    pub class: Class,
    /// The shown text.
    pub text: String,
    pub number: Option<f64>,
    pub style: Style,
    pub formula: Option<FormulaIn>,
}

impl CellIn {
    /// Whether the cell holds a position for the cut: some text, or a formula.
    pub fn occupied(&self) -> bool {
        !self.text.is_empty() || self.formula.is_some()
    }
}

/// A layout fact the file states for a rectangle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactKind {
    /// An Excel table.
    ListObject {
        name: String,
        header_rows: u32,
        totals_rows: u32,
        columns: Vec<String>,
    },
    /// An ODS database range.
    DatabaseRange {
        name: String,
        header: bool,
        footer: bool,
    },
    /// A pivot table output range. The three offsets count from the range's top row and left
    /// column.
    Pivot {
        name: String,
        first_header_row: u32,
        first_data_row: u32,
        first_data_col: u32,
        total_row: bool,
        total_col: bool,
    },
    /// An autofilter range; its first row is the header.
    AutoFilter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    pub rect: Rect,
    pub kind: FactKind,
}

/// A defined name whose whole text is one rectangle on this sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameIn {
    pub name: String,
    pub rect: Rect,
    pub hidden: bool,
    /// A name the application writes for itself (`_xlnm.*`).
    pub builtin: bool,
}

/// One sheet as the caller read it.
pub struct Input<'a> {
    /// Cells with content, sorted by (row, col), one per position. Unsorted or repeated cells
    /// are tolerated: the first cell at a position counts.
    pub cells: Vec<CellIn>,
    pub merges: Vec<Rect>,
    pub facts: Vec<Fact>,
    pub names: Vec<NameIn>,
    /// Row outline groups: first row, last row, level.
    pub outline: Vec<(u32, u32, u8)>,
    /// Frozen panes: the number of rows and columns above and left of the split.
    pub frozen: Option<(u32, u32)>,
    /// Rows repeated on every printed page: first and last row.
    pub print_titles: Option<(u32, u32)>,
    /// A header guess from elsewhere (CSV): the row, and whether it is a header.
    pub header_guess: Option<(u32, bool)>,
    /// Whether the style flags were read at all.
    pub styles_read: bool,
    /// Whether a text reads as a label (for label and value blocks).
    pub is_label: &'a dyn Fn(&str) -> bool,
    /// Whether the texts of a column read as units.
    pub is_unit_column: &'a dyn Fn(&[&str]) -> bool,
}

/// Work limits per sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Levels of the recursive cut.
    pub max_cut_depth: u32,
    /// Regions per sheet, the rest region included.
    pub max_regions: usize,
    /// Depth of the section recursion of report sheets.
    pub max_section_depth: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_cut_depth: 64,
            max_regions: 10_000,
            max_section_depth: 8,
        }
    }
}
