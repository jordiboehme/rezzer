// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The start of the loss a spreadsheet reader writes when the stored results may not match
/// the formulas (`stored results may be old: <reasons>`). The renderer finds the loss by it
/// and shows it as a warning at the top of the Logic section.
pub const STALE_RESULTS: &str = "stored results may be old";

/// The words in the middle of the loss a paged reader writes for a region it keeps as laid
/// out because no region type is clear for it (`the region at p1:b3 is kept as laid out,
/// line by line: ...`). Such a loss only says how the text was read: the renderer leaves it
/// out at the note level `brief` when it is uncertain and of severity info. A region a limit
/// left whole has the reason limit and is always shown.
pub const KEPT_AS_LAID_OUT: &str = " is kept as laid out, ";

/// Why a region is kept as laid out when no region type reaches the least score: the words
/// after the colon of its [`KEPT_AS_LAID_OUT`] loss.
pub const LAID_OUT_NO_EVIDENCE: &str = "no region type has clear evidence for it";

/// The words of a [`KEPT_AS_LAID_OUT`] loss whose region reads as two kinds about equally
/// (`it reads as heading or as paragraph about equally (410 and 400 of 1000)`).
pub const LAID_OUT_ABOUT_EQUALLY: &str = " about equally (";

/// The words of a [`KEPT_AS_LAID_OUT`] loss whose region reads as one kind, but not clearly
/// enough (`it reads as table (350 of 1000), but not clearly enough`).
pub const LAID_OUT_NOT_CLEARLY: &str = ", but not clearly enough";

/// The end of a [`KEPT_AS_LAID_OUT`] loss whose region holds bold or italic text that its
/// Markdown does not show (`; its bold text is marked in the JSON only`).
pub const MARKED_IN_JSON_ONLY: &str = " text is marked in the JSON only";

/// The end of the footnote note of a page whose one footnote is linked to its marker.
pub const FOOTNOTE_LINKED: &str = "; the marker above is linked to it";

/// The end of the footnote note of a page whose footnotes are all linked to their markers.
pub const FOOTNOTES_LINKED: &str = "; the markers above are linked to them";

/// The role of a paragraph that says how a paged reader found what follows it on the page
/// (a "Found by:" note). It is no text of the file.
pub const FOUND_BY_ROLE: &str = "found-by";

/// The found-by note before the first table of drawn boxes of a page.
pub const BOX_FOUND_BY: &str = "Found by: label and value pairs from drawn boxes; the label \
     is the smaller text at the top of each box.";

/// The found-by note before the first table of fields between rules of a page.
pub const STRIP_FOUND_BY: &str = "Found by: label and value pairs from rows of fields \
     between drawn rules; each label is the smaller text and pairs with the value beside it or \
     under it.";

/// The found-by note before the first table on a page that a column of totals adds to.
pub const SIDE_FOUND_BY: &str = "Found by: a column of amounts beside a table, drawn with \
     rules that line up with the table's rules; its amounts are read as one more column of the \
     table, and each total label in it pairs with the amount under it.";

/// How a footnote legend is found: the words of the footnote note after its count.
pub const FOOTNOTE_FOUND_BY: &str = "set in one size below all other text, each opening \
     with a note marker, the first of which also stands after a word above";

/// Something the reader could not read, with the reason. Rezzer never drops content silently.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct Loss {
    /// An anchor or a description of where the loss is.
    pub location: String,
    pub what: String,
    pub why: LossReason,
    pub severity: Severity,
}

impl Loss {
    /// True when the loss says something was not read: every loss but an uncertain one. An
    /// uncertain loss (an unsure region, a guessed dialect) was read, only not surely.
    pub fn is_not_read(&self) -> bool {
        self.why != LossReason::Uncertain
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LossReason {
    Unsupported,
    Damaged,
    Limit,
    Encrypted,
    NotImplemented,
    /// A fact was guessed or inferred, not read from the file.
    Uncertain,
    /// Rezzer itself stopped with an internal error (a reader or enrich pass panicked): a bug
    /// in Rezzer, not a fault of the file.
    Internal,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}
