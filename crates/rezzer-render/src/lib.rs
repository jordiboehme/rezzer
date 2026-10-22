// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The one place that turns the document model into OKF Markdown with Crystalline's dialect,
//! Mermaid graphs and paging. Every format's output goes through it.
//!
//! Output order: frontmatter, `# <title>`, `## Overview`, `## Properties`, the content,
//! `## Logic`, `## Annotations`, `## Changes`, `## Embedded`, `## Not read`. Empty sections are left out.
//!
//! Decisions:
//! - Text from files is escaped so it never adds structure, HTML or frontmatter
//! - Anchors are not printed. A heading anchor is the heading's slug already; every other
//!   anchor stays in the model JSON (`--json`). Footnote sections (`fn:<label>`) become
//!   footnote definitions
//! - A hard line break is a backslash at the line end
//! - An annotation stands at its passage: after the first block with its anchor, or at the
//!   end of its page section. Only the others are listed in `## Annotations`
//! - A leading level-1 heading equal to the title is not repeated; body headings shift down
//!   so none is level 1. Titled sections get `##`, nested ones `###` and so on

mod blocks;
mod facets;
mod form;
mod frontmatter;
mod mermaid;
mod overview;
mod paging;
mod workbook;

use rezzer_model::{Block, Completeness, Document, plain_text};

use crate::blocks::{Ctx, MAX_DEPTH, Notes};

/// A part of the output that a caller can ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputSection {
    Overview,
    /// Custom document properties.
    Properties,
    Content,
    Logic,
    Annotations,
    Changes,
    Embedded,
    NotRead,
}

impl OutputSection {
    /// Every section in output order.
    pub const ALL: [OutputSection; 8] = [
        OutputSection::Overview,
        OutputSection::Properties,
        OutputSection::Content,
        OutputSection::Logic,
        OutputSection::Annotations,
        OutputSection::Changes,
        OutputSection::Embedded,
        OutputSection::NotRead,
    ];

    /// The snake_case name callers use, for example `not_read`.
    pub fn name(self) -> &'static str {
        match self {
            OutputSection::Overview => "overview",
            OutputSection::Properties => "properties",
            OutputSection::Content => "content",
            OutputSection::Logic => "logic",
            OutputSection::Annotations => "annotations",
            OutputSection::Changes => "changes",
            OutputSection::Embedded => "embedded",
            OutputSection::NotRead => "not_read",
        }
    }
}

impl std::str::FromStr for OutputSection {
    type Err = String;

    /// Takes the [`OutputSection::name`]; case, surrounding spaces and `-` for `_` are allowed.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let wanted = s.trim().to_ascii_lowercase().replace('-', "_");
        OutputSection::ALL
            .into_iter()
            .find(|section| section.name() == wanted)
            .ok_or_else(|| {
                let names: Vec<&str> = OutputSection::ALL.iter().map(|s| s.name()).collect();
                format!(
                    "unknown section {:?}, expected one of {}",
                    s.trim(),
                    names.join(", ")
                )
            })
    }
}

/// The snake_case name of a format, as in the model JSON and the frontmatter, for example
/// `three_mf`.
pub fn format_name(format: rezzer_model::FormatId) -> String {
    frontmatter::format_name(format)
}

#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// The sections to write; `None` writes all. Frontmatter and title are always written.
    pub sections: Option<Vec<OutputSection>>,
    /// RFC 3339 time for `generated.at`, injected so output is deterministic.
    pub generated_at: String,
    pub rezzer_version: String,
    /// `rezzer://sha256/<hash>/<name>` when the archive is on.
    pub archive_ref: Option<String>,
    /// Show formulas also as LaTeX math (the `Math` column of `## Logic`). Default true.
    pub math: bool,
}

impl Default for RenderOptions {
    fn default() -> RenderOptions {
        RenderOptions {
            sections: None,
            generated_at: String::new(),
            rezzer_version: String::new(),
            archive_ref: None,
            math: true,
        }
    }
}

/// Renders a document as OKF Markdown.
pub fn render(doc: &Document, completeness: Completeness, opts: &RenderOptions) -> String {
    let want = |s: OutputSection| opts.sections.as_ref().is_none_or(|l| l.contains(&s));
    let mut out = frontmatter::render(doc, completeness, opts);
    let title = frontmatter::title(doc).trim();
    out.push('\n');
    // The heading that became the title keeps its links.
    let title_line = match doc.body.first() {
        Some(Block::Heading {
            level: 1, inlines, ..
        }) if plain_text(inlines).trim() == title => blocks::inlines(inlines, blocks::Mode::Line)
            .trim()
            .to_string(),
        _ => blocks::line_text(title),
    };
    out.push_str(&blocks::heading_line(1, &title_line));
    out.push('\n');

    let mut parts: Vec<String> = Vec::new();
    if want(OutputSection::Overview)
        && let Some(overview) = render_overview(doc)
    {
        parts.push(overview);
    }
    if want(OutputSection::Properties) {
        parts.extend(facets::properties(doc));
    }
    let notes = Notes::new(&doc.facets.annotations);
    if want(OutputSection::Content) {
        parts.push(content(doc, title, &notes));
    }
    if want(OutputSection::Logic) {
        parts.extend(facets::logic(doc, opts.math));
    }
    let ctx = Ctx {
        codes_shown: Default::default(),
        assets: &doc.assets,
        heading_offset: 0,
        notes: None,
    };
    if want(OutputSection::Annotations) {
        parts.extend(facets::annotations(doc, &ctx, &notes));
    }
    if want(OutputSection::Changes) {
        parts.extend(facets::changes(doc));
    }
    if want(OutputSection::Embedded) {
        parts.extend(facets::embedded(doc));
    }
    if want(OutputSection::NotRead) {
        parts.extend(facets::not_read(doc));
    }
    // One growth for all parts, so a huge part is not doubled by the push.
    out.reserve_exact(
        parts
            .iter()
            .map(|p| p.len() + if p.is_empty() { 0 } else { 2 })
            .sum(),
    );
    for part in parts.iter().filter(|p| !p.is_empty()) {
        out.push('\n');
        out.push_str(part);
        out.push('\n');
    }
    out
}

/// `## Overview`: the parts with kind, size, role and what they read from, the facts, and a
/// Mermaid graph of the parts when one reads from another. `None` when the document has no
/// overview (the enrich pass `overview` did not run) or it is empty.
pub fn render_overview(doc: &Document) -> Option<String> {
    overview::render(doc)
}

/// Splits rendered Markdown into pages of about `page_tokens` tokens (characters / 4).
///
/// Pages break only before a `## ` heading; a section larger than a page breaks before its
/// `### ` headings, else at a blank line. Never inside a fence or a table; the frontmatter
/// and the title stay on page 1. `page_tokens == 0` means one page. The pages joined are the
/// input.
pub fn paginate(markdown: &str, page_tokens: usize) -> Vec<String> {
    paging::paginate(markdown, page_tokens)
}

fn content(doc: &Document, title: &str, notes: &Notes) -> String {
    let mut body: &[Block] = &doc.body;
    if let Some(Block::Heading {
        level: 1, inlines, ..
    }) = body.first()
        && plain_text(inlines).trim() == title
    {
        body = &body[1..];
    }
    let min = min_heading(body, 0).unwrap_or(2).max(1);
    let ctx = Ctx {
        codes_shown: Default::default(),
        assets: &doc.assets,
        heading_offset: 2u8.saturating_sub(min),
        notes: Some(notes),
    };
    blocks::blocks(body, &ctx, 0, 2, false)
}

/// The smallest heading level anywhere in `list`.
fn min_heading(list: &[Block], depth: usize) -> Option<u8> {
    if depth > MAX_DEPTH {
        return None;
    }
    let mut min: Option<u8> = None;
    let mut take = |l: Option<u8>| {
        if let Some(l) = l {
            min = Some(min.map_or(l, |m: u8| m.min(l)));
        }
    };
    for b in list {
        match b {
            Block::Heading { level, .. } => take(Some(*level)),
            Block::Section { blocks, .. }
            | Block::Quote { blocks, .. }
            | Block::Callout { blocks, .. } => take(min_heading(blocks, depth + 1)),
            Block::List { items, .. } => {
                for item in items {
                    take(min_heading(item, depth + 1));
                }
            }
            _ => {}
        }
    }
    min
}
