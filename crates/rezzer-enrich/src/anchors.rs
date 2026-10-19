// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Pass `anchors`: every reference to an anchor must name an anchor of the document.
//!
//! The anchors of a document are the anchors of its blocks (sections included), table cells,
//! `AnchorMark` inlines and graph nodes. A reference resolves when it is one of them. A
//! spreadsheet reference may also name a range or a whole column or row, such as `Sheet!A:A`
//! or `Sheet!A1:C9`: it resolves when its sheet prefix (the text before the `!`, without
//! single quotes) equals the anchor of a section. Everything else that is missing becomes a
//! Warning loss.

use std::collections::HashSet;

use rezzer_model::{Anchor, Block, Document, Inline, Loss, LossReason, Severity};

use crate::{Pass, walk};

/// At most this many losses are written; one more loss says how many were left out.
const MAX_LOSSES: usize = 100;

pub(crate) struct Anchors;

impl Pass for Anchors {
    fn name(&self) -> &'static str {
        "anchors"
    }

    fn run(&self, doc: &mut Document) {
        let set = AnchorSet::of(doc);
        let mut missing: Vec<(String, String)> = Vec::new();
        let mut check = |location: String, anchor: &Anchor| {
            if !set.resolves(&anchor.0) {
                missing.push((location, anchor.0.clone()));
            }
        };
        for f in &doc.facets.formulas {
            check(f.anchor.0.clone(), &f.anchor);
            for r in &f.references {
                check(f.anchor.0.clone(), r);
            }
        }
        for g in &doc.facets.graphs {
            for e in &g.edges {
                if let Some(a) = &e.anchor {
                    check(format!("graph {}", g.id), a);
                }
            }
        }
        let mut notes: Vec<&rezzer_model::Annotation> = doc.facets.annotations.iter().collect();
        while let Some(n) = notes.pop() {
            if let Some(a) = &n.anchor {
                check(format!("annotation ({})", n.kind), a);
            }
            notes.extend(n.replies.iter());
        }
        for e in &doc.facets.timeline {
            if let Some(a) = &e.anchor {
                check(format!("change ({})", e.kind), a);
            }
        }
        let total = missing.len();
        for (location, anchor) in missing.into_iter().take(MAX_LOSSES) {
            doc.losses.push(Loss {
                location,
                what: format!("reference to {anchor} not found"),
                why: LossReason::Damaged,
                severity: Severity::Warning,
            });
        }
        if total > MAX_LOSSES {
            doc.losses.push(Loss {
                location: "document".into(),
                what: format!(
                    "{} more references to missing anchors are not listed",
                    total - MAX_LOSSES
                ),
                why: LossReason::Damaged,
                severity: Severity::Warning,
            });
        }
    }
}

/// The anchors of one document.
pub(crate) struct AnchorSet<'a> {
    all: HashSet<&'a str>,
    sections: HashSet<&'a str>,
}

impl<'a> AnchorSet<'a> {
    pub(crate) fn of(doc: &'a Document) -> AnchorSet<'a> {
        let mut set = AnchorSet {
            all: HashSet::new(),
            sections: HashSet::new(),
        };
        set.add_blocks(&doc.body);
        for g in &doc.facets.graphs {
            for n in &g.nodes {
                if let Some(a) = &n.anchor {
                    set.all.insert(&a.0);
                }
            }
        }
        set
    }

    fn add_blocks(&mut self, body: &'a [Block]) {
        let mut found: Vec<&'a Anchor> = Vec::new();
        let mut sections: Vec<&'a Anchor> = Vec::new();
        collect(body, &mut found, &mut sections);
        self.all.extend(found.into_iter().map(|a| a.0.as_str()));
        self.sections
            .extend(sections.into_iter().map(|a| a.0.as_str()));
    }

    /// True when `anchor` is an anchor of the document, or a range on a sheet that is.
    pub(crate) fn resolves(&self, anchor: &str) -> bool {
        if self.all.contains(anchor) {
            return true;
        }
        match sheet_prefix(anchor) {
            Some(sheet) => self.sections.contains(sheet.as_str()),
            None => false,
        }
    }
}

/// Collects every anchor of `body` into `found` (blocks, cells, `AnchorMark` inlines) and the
/// anchors of its sections also into `sections`.
pub(crate) fn collect<'a>(
    body: &'a [Block],
    found: &mut Vec<&'a Anchor>,
    sections: &mut Vec<&'a Anchor>,
) {
    walk::blocks(body, |b| {
        if let Block::Section {
            anchor: Some(a), ..
        } = b
        {
            sections.push(a);
        }
        if let Some(a) = block_anchor(b) {
            found.push(a);
        }
        if let Block::Table { rows, .. } = b {
            found.extend(rows.iter().flatten().filter_map(|c| c.anchor.as_ref()));
        }
        walk::block_inlines(b, |list| {
            walk::inlines(list, |i| {
                if let Inline::AnchorMark { anchor } = i {
                    found.push(anchor);
                }
            });
        });
        true
    });
}

/// The anchor of any block variant.
pub(crate) fn block_anchor(block: &Block) -> Option<&Anchor> {
    match block {
        Block::Section { anchor, .. }
        | Block::Heading { anchor, .. }
        | Block::Paragraph { anchor, .. }
        | Block::List { anchor, .. }
        | Block::Table { anchor, .. }
        | Block::Code { anchor, .. }
        | Block::Quote { anchor, .. }
        | Block::Math { anchor, .. }
        | Block::Media { anchor, .. }
        | Block::Callout { anchor, .. }
        | Block::Rule { anchor }
        | Block::Task { anchor, .. } => anchor.as_ref(),
    }
}

/// The sheet name of `Sheet!A1`, `'My Sheet'!A:A` and so on; quotes removed, `''` read as `'`.
pub(crate) fn sheet_prefix(reference: &str) -> Option<String> {
    if let Some(rest) = reference.strip_prefix('\'') {
        let mut name = String::new();
        let mut chars = rest.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c == '\'' {
                if matches!(chars.peek(), Some((_, '\''))) {
                    name.push('\'');
                    chars.next();
                } else {
                    return rest[i + 1..].starts_with('!').then_some(name);
                }
            } else {
                name.push(c);
            }
        }
        return None;
    }
    reference
        .split_once('!')
        .map(|(sheet, _)| sheet.to_string())
        .filter(|s| !s.is_empty())
}
