// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Print-run forms: a form template with a data layer of padded fixed-pitch lines, built in
//! code with invented values only.

#[path = "../common/mod.rs"]
mod common;

mod boxes;
mod cells;
mod dates;
mod footnotes;
mod grids;
mod headings;
mod marked;
mod marks;
mod padded;
mod pay_slip;
mod rows;
mod sections;
mod text;

use std::time::{Duration, Instant};

use dates::*;
use grids::*;
use headings::*;
use marked::*;
use marks::*;
use padded::*;
use pay_slip::*;
use sections::*;

use common::{DataLine, FormPage, all_text, boxed, data_line, in_order, label, mark, print_form};
use rezzer_model::{Block, Completeness, Inline, LossReason, ReadOutcome, Severity, plain_text};

/// The allocator of the Rezzer binary, so the reads run as they run in it.
#[global_allocator]
static ALLOCATOR: rezzer_limits::Allocator = rezzer_limits::Allocator;

/// `bytes` read with the default options and limits. Every read of this file also checks
/// that no placeholder of Rezzer reaches the Markdown or the JSON.
fn read(bytes: &[u8]) -> ReadOutcome {
    let out = common::read(bytes);
    assert_no_placeholder(&out);
    out
}

/// Asserts that neither the rendered Markdown nor the JSON of the model of `out` holds a
/// noncharacter of U+FDD0 to U+FDEF: those are Rezzer's own placeholders while a page is read.
fn assert_no_placeholder(out: &ReadOutcome) {
    let md = rezzer_render::render(
        &out.document,
        out.completeness,
        &rezzer_render::RenderOptions::default(),
    );
    // JSON writes the noncharacters as they are, not escaped.
    let json = serde_json::to_string(&out.document).expect("json");
    let placeholder = |c: char| ('\u{FDD0}'..='\u{FDEF}').contains(&c);
    assert!(
        !md.contains(placeholder),
        "a placeholder in the Markdown: {md}"
    );
    assert!(!json.contains(placeholder), "a placeholder in the JSON");
}

/// The `Datum` of the header field grid, a label over its value in a row of fields, is the
/// document date: the pairs of the grid say it.
fn assert_grid_date(out: &ReadOutcome) {
    let found = out.document.properties.document_date.as_ref();
    assert_eq!(
        found.map(|d| (d.date.as_str(), d.from.as_str())),
        Some(("2026-10-01", "date label \"Datum\" on page 1"))
    );
}

/// The read is complete and names no loss.
fn assert_no_loss(out: &ReadOutcome) {
    assert_eq!(
        out.completeness,
        Completeness::Complete,
        "{:?}",
        out.document.losses
    );
    assert!(out.document.losses.is_empty(), "{:?}", out.document.losses);
}

/// The read is complete and names no loss but the one for the date in the head that nothing
/// marks as the document date (the `Eintritt` field of the boxed fields).
fn assert_no_loss_but_the_head_date(out: &ReadOutcome) {
    assert_eq!(
        out.completeness,
        Completeness::Complete,
        "{:?}",
        out.document.losses
    );
    let what: Vec<&str> = out
        .document
        .losses
        .iter()
        .map(|l| l.what.as_str())
        .collect();
    assert_eq!(what, [ONE_DATE], "{:?}", out.document.losses);
}
