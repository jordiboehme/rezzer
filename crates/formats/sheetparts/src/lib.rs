// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Workbook parts no spreadsheet crate exposes well.
//!
//! A spreadsheet library gives you cells. This crate reads what tells you what the cells mean:
//! sheet names and visibility, defined names, calculation settings, number formats, document
//! properties, external links and the people behind comments.
//!
//! Every part is parsed from untrusted XML with a bound reader: a DOCTYPE is refused, only the
//! five predefined entities are known, and depth and item counts are limited. A problem never
//! panics and never stops the other parts. It becomes an [`Issue`] in the result.
//!
//! ```
//! use sheetparts::numfmt;
//! assert_eq!(numfmt::format("0%", 0.09).text, "9%");
//! assert_eq!(numfmt::builtin(14), Some("mm-dd-yy"));
//! ```

mod a1;
pub mod numfmt;

pub use a1::quote_sheet;
pub mod odf;
pub mod ooxml;
pub(crate) mod xml;

/// Bounds for reading one part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Deepest element nesting a part may have.
    pub max_depth: usize,
    /// Most XML events (tags, text runs and attributes) in one part.
    pub max_items: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_depth: 256,
            max_items: 2_000_000,
        }
    }
}

/// Why a part, or a piece of it, could not be read in full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueKind {
    /// The part is broken or points at something that is not there.
    Damaged,
    /// A limit was hit or the caller asked to stop.
    Limit,
    /// The part uses something that is not read, on purpose.
    Unsupported,
}

/// One thing that went wrong while reading, with the part it concerns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// The part name in the package, for example `xl/workbook.xml`.
    pub part: String,
    /// What happened, in plain words.
    pub what: String,
    /// The class of the problem.
    pub kind: IssueKind,
}

impl Issue {
    pub(crate) fn new(part: &str, kind: IssueKind, what: impl Into<String>) -> Issue {
        Issue {
            part: part.to_string(),
            what: what.into(),
            kind,
        }
    }
}

/// A cell position, 0-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellPos {
    pub row: u32,
    pub col: u32,
}

/// A run of rows, 0-based and inclusive at both ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowSpan {
    pub first: u32,
    pub last: u32,
}

/// The value of a custom document property, keeping the type the file gave it.
#[derive(Debug, Clone, PartialEq)]
pub enum PropValue {
    Text(String),
    Integer(i64),
    Number(f64),
    Bool(bool),
    /// An ISO 8601 date or date and time, as written in the file.
    DateTime(String),
    /// Any other `vt:` type, with its text kept as is.
    Other {
        ty: String,
        value: String,
    },
}

/// A defined name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinedName {
    pub name: String,
    /// The sheet name when the name is local to one sheet, `None` for a global name.
    pub scope: Option<String>,
    /// The formula text as written, for example `Sheet1!$A$1` or `{"A","B"}`.
    pub refers_to: String,
    pub hidden: bool,
    pub comment: Option<String>,
}
