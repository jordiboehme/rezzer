// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The path trace: which path of the reader made the region that holds a text. It prints the
//! region's kind, how sure it is, the ids of its evidence and its bounds, and no text of the
//! file, so that a trace of a private file may be passed on.

use rezzer_model::{Certainty, Document, Evidence, RegionNote};

use crate::check::kind_name;
use crate::doc::View;
use crate::text::{has_words, norm};

/// The id of one piece of evidence. Evidence that carries a name of the file gives only its
/// kind.
fn evidence_id(evidence: &Evidence) -> &str {
    match evidence {
        Evidence::Rule { id, .. } => id,
        Evidence::ListObject { .. } => "list_object",
        Evidence::DatabaseRange { .. } => "database_range",
        Evidence::Pivot { .. } => "pivot",
        Evidence::Autofilter => "autofilter",
        Evidence::PrintTitles => "print_titles",
        Evidence::FrozenPanes => "frozen_panes",
        Evidence::DefinedName { .. } => "defined_name",
    }
}

/// The anchor as the trace prints it: the label of a footnote is text of the file and is
/// left out.
fn shown(anchor: &str) -> &str {
    if anchor.starts_with("fn:") {
        "fn:"
    } else {
        anchor
    }
}

/// One line of the trace for `region`.
fn line(region: &RegionNote) -> String {
    let certainty = match region.certainty {
        Certainty::Exact => "exact",
        Certainty::Rule => "rule",
        Certainty::Unsure => "unsure",
    };
    let ids: Vec<&str> = region.evidence.iter().map(evidence_id).collect();
    let also: Vec<String> = region
        .alternatives
        .iter()
        .map(|a| format!("{}:{}", kind_name(a.kind), a.score))
        .collect();
    let [x0, y0, x1, y1] = region.bounds;
    format!(
        "{}\tpage {}\t{}\t{certainty}\tscore {}\tevidence {}\tbounds {x0} {y0} {x1} {y1}\talso {}",
        shown(&region.anchor.0),
        region.page,
        kind_name(region.kind),
        region.score,
        if ids.is_empty() {
            "-".to_string()
        } else {
            ids.join(",")
        },
        if also.is_empty() {
            "-".to_string()
        } else {
            also.join(",")
        },
    )
}

/// The trace of `what` in `doc`: when `what` is the anchor of a region, the regions at that
/// anchor; else the regions of the blocks whose text holds `what`, one line each. A block
/// that holds the text and has no region gives `no region`. Empty when nothing is found.
pub fn trace(doc: &Document, what: &str) -> Vec<String> {
    let at: Vec<String> = doc
        .regions
        .iter()
        .filter(|r| r.anchor.0 == what)
        .map(line)
        .collect();
    if !at.is_empty() {
        return at;
    }
    let view = View::of(doc);
    let what = norm(what);
    let mut anchors: Vec<Option<&str>> = Vec::new();
    for (n, unit) in view.units.iter().enumerate() {
        if has_words(&unit.text, &what) {
            let anchor = view.region_of(n).map(|r| r.anchor.0.as_str());
            if !anchors.contains(&anchor) {
                anchors.push(anchor);
            }
        }
    }
    let mut out = Vec::new();
    for anchor in anchors {
        match anchor {
            Some(anchor) => out.extend(
                doc.regions
                    .iter()
                    .filter(|r| r.anchor.0 == anchor)
                    .map(line),
            ),
            None => out.push("no region".to_string()),
        }
    }
    out
}
