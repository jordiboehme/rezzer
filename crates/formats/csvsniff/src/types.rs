// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Public result types of the sniffer.

/// A detected CSV dialect.
#[derive(Debug, Clone, PartialEq)]
pub struct Dialect {
    /// Field delimiter, one of `,` `;` tab or `|`.
    pub delimiter: u8,
    /// Quote character (always `"` for now).
    pub quote: u8,
    /// Whether the first record looks like a header row. This is a heuristic;
    /// for header-less files it is a guess and callers should say so.
    pub has_header: bool,
    /// Share (0.0 to 1.0) of the sampled records that have the modal field count.
    pub consistent_rows: f32,
}

/// The inferred type of a column, computed over its non-empty cells.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnKind {
    /// Whole numbers.
    Integer,
    /// Numbers with a fraction; `places` is the largest number of decimal places seen.
    Decimal {
        /// Largest number of digits after the decimal point.
        places: u8,
    },
    /// Calendar dates (ISO, day.month.year, month/day/year, "31-May-13", with optional time).
    Date,
    /// `true`/`false`/`yes`/`no` values.
    Bool,
    /// Anything else, including mixed columns.
    Text,
    /// No non-empty cell at all.
    Empty,
}

/// Facts about one column.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    /// Header text, or `Column N` (1-based) when there is no header or the cell is empty.
    pub name: String,
    /// Inferred type.
    pub kind: ColumnKind,
    /// Unit taken from the header: `(kg)`, `[m]` or a trailing `in %`.
    pub unit: Option<String>,
    /// Number of empty cells (rows too short for this column count as empty).
    pub missing: usize,
    /// True when no non-empty value occurs twice. False can also mean the
    /// uniqueness budget ran out; see [`Column::unique_exhausted`].
    pub unique: bool,
    /// True when `unique` is false only because the per-column (200,000) or
    /// per-file (2,000,000) limit on tracked values was reached, not because
    /// a duplicate was seen.
    pub unique_exhausted: bool,
    /// Unique, no missing value, at least two rows, and Integer or short Text.
    pub key_like: bool,
}

/// A record whose field count differs from the modal field count.
#[derive(Debug, Clone, PartialEq)]
pub struct RaggedRow {
    /// 1-based physical line on which the record starts.
    pub line: usize,
    /// Fields found.
    pub fields: usize,
    /// Fields expected (the modal count).
    pub expected: usize,
}

/// The operation of a derived column.
#[derive(Debug, Clone, PartialEq)]
pub enum DerivedOp {
    /// The target is the sum of the parts.
    Sum,
}

/// An observed relation: a column that equals the sum of other columns,
/// within rounding, on most rows. It is an observation, not a proof.
#[derive(Debug, Clone, PartialEq)]
pub struct Derived {
    /// Column index of the total.
    pub target: usize,
    /// Column indexes of the parts, ascending.
    pub parts: Vec<usize>,
    /// The operation.
    pub op: DerivedOp,
    /// Rows on which the relation holds within tolerance.
    pub holds: usize,
    /// Rows that were checked (no missing value, non-zero target).
    pub checked: usize,
}

/// Result of [`crate::analyze`].
#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    /// True when [`crate::analyze_until`] was stopped early: the counts and column facts cover
    /// only the records read so far.
    pub truncated: bool,
    /// One entry per column (the modal field count; extra cells in longer records are not columns).
    pub columns: Vec<Column>,
    /// Number of data rows (the header is not counted).
    pub rows: usize,
    /// Records with an unexpected field count; the first 1,000 only.
    pub ragged_rows: Vec<RaggedRow>,
    /// Total number of records with an unexpected field count.
    pub ragged_total: usize,
    /// True when a quoted field is still open at the end of the text.
    pub unterminated_quote: bool,
    /// Quoted fields that are not closed properly (see [`QuoteProblem`]): the first one in the
    /// middle of the text, if any, and one that is still open at the end, if any.
    pub quote_problems: Vec<QuoteProblem>,
    /// Observed sum relations between numeric columns.
    pub derived: Vec<Derived>,
}

/// A quoted field that is not closed properly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuoteProblem {
    /// Byte offset of the opening quote in the analyzed text.
    pub offset: usize,
    /// True when the field is still open at the end of the text (the rest of the text is one
    /// field); false when it runs over line breaks and ends in a quote that is not at the end of
    /// a field.
    pub at_end: bool,
}
