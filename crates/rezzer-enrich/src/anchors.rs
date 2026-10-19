// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Pass `anchors`: every reference to an anchor must name an anchor of the document.
//!
//! The anchors of a document are the anchors of its blocks (sections included), table cells,
//! `AnchorMark` inlines. Graph node
//! anchors are references into the body, not definitions. A reference resolves when it is one
//! of them. A
//! spreadsheet reference may also name a range or a whole column or row, such as `Sheet!A:A`
//! or `Sheet!A1:C9`: it resolves when its sheet prefix (the text before the `!`, without
//! single quotes; every `!` split is tried) equals the anchor of a section. Everything else that is missing becomes a
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
            for n in &g.nodes {
                if let Some(a) = &n.anchor {
                    check(format!("graph {}", g.id), a);
                }
            }
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
        let mut new: Vec<Loss> = missing
            .into_iter()
            .take(MAX_LOSSES)
            .map(|(location, anchor)| Loss {
                location,
                what: format!("reference to {anchor} not found"),
                why: LossReason::Damaged,
                severity: Severity::Warning,
            })
            .collect();
        if total > MAX_LOSSES {
            new.push(Loss {
                location: "document".into(),
                what: format!(
                    "{} more references to missing anchors are not listed",
                    total - MAX_LOSSES
                ),
                why: LossReason::Damaged,
                severity: Severity::Warning,
            });
        }
        // A loss equal to one already in the document is not added again, so the pass can run
        // twice.
        for loss in new {
            if !doc.losses.contains(&loss) {
                doc.losses.push(loss);
            }
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
        sheet_prefixes(anchor)
            .iter()
            .any(|sheet| self.sections.contains(sheet.as_str()))
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

/// The possible sheet names of a reference like `Sheet!A1`, longest first. A sheet name may
/// itself contain `!` or start with `'` (a file name), and readers write it unquoted, so every
/// `!` split is a candidate. A quoted name (`'My Sheet'!A:A`, `''` read as `'`) is one more.
pub(crate) fn sheet_prefixes(reference: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(rest) = reference.strip_prefix('\'') {
        let mut name = String::new();
        let mut chars = rest.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c == '\'' {
                if matches!(chars.peek(), Some((_, '\''))) {
                    name.push('\'');
                    chars.next();
                } else {
                    if rest[i + 1..].starts_with('!') {
                        out.push(name.clone());
                    }
                    break;
                }
            } else {
                name.push(c);
            }
        }
    }
    for (i, _) in reference
        .match_indices('!')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        if i > 0 {
            out.push(reference[..i].to_string());
        }
    }
    out.sort_by_key(|p| std::cmp::Reverse(p.len()));
    out.dedup();
    out
}
