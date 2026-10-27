// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The start of the loss a spreadsheet reader writes when the stored results may not match
/// the formulas (`stored results may be old: <reasons>`). The renderer finds the loss by it
/// and shows it as a warning at the top of the Logic section.
pub const STALE_RESULTS: &str = "stored results may be old";

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
