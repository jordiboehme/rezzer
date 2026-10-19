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
//! - A leading level-1 heading equal to the title is not repeated; body headings shift down
//!   so none is level 1. Titled sections get `##`, nested ones `###` and so on

mod blocks;
mod facets;
mod frontmatter;
mod mermaid;
mod paging;

use rezzer_model::{Block, Completeness, Document, Inline};

use crate::blocks::{Ctx, MAX_DEPTH};

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

#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    /// The sections to write; `None` writes all. Frontmatter and title are always written.
    pub sections: Option<Vec<OutputSection>>,
    /// RFC 3339 time for `generated.at`, injected so output is deterministic.
    pub generated_at: String,
    pub rezzer_version: String,
    /// `rezzer://sha256/<hash>/<name>` when the archive is on.
    pub archive_ref: Option<String>,
}

/// Renders a document as OKF Markdown.
pub fn render(doc: &Document, completeness: Completeness, opts: &RenderOptions) -> String {
    let want = |s: OutputSection| opts.sections.as_ref().is_none_or(|l| l.contains(&s));
    let mut out = frontmatter::render(doc, completeness, opts);
    let title = frontmatter::title(doc).trim();
    out.push('\n');
    out.push_str(&blocks::heading_line(1, &blocks::line_text(title)));
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
    if want(OutputSection::Content) {
        parts.push(content(doc, title));
    }
    if want(OutputSection::Logic) {
        parts.extend(facets::logic(doc));
    }
    let ctx = Ctx {
        assets: &doc.assets,
        heading_offset: 0,
    };
    if want(OutputSection::Annotations) {
        parts.extend(facets::annotations(doc, &ctx));
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
    for part in parts.iter().filter(|p| !p.is_empty()) {
        out.push('\n');
        out.push_str(part);
        out.push('\n');
    }
    out
}

/// `## Overview`: parts with roles, counts, dependencies, last change and open comments.
/// Task 11 (enrich) fills it; until then there is none.
pub fn render_overview(_doc: &Document) -> Option<String> {
    None
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

fn content(doc: &Document, title: &str) -> String {
    let mut body: &[Block] = &doc.body;
    if let Some(Block::Heading {
        level: 1, inlines, ..
    }) = body.first()
        && plain_text(inlines, 0).trim() == title
    {
        body = &body[1..];
    }
    let min = min_heading(body, 0).unwrap_or(2).max(1);
    let ctx = Ctx {
        assets: &doc.assets,
        heading_offset: 2u8.saturating_sub(min),
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

fn plain_text(list: &[Inline], depth: usize) -> String {
    let mut out = String::new();
    if depth > MAX_DEPTH {
        return out;
    }
    for i in list {
        match i {
            Inline::Text { text, .. } | Inline::Code { text } => out.push_str(text),
            Inline::Link { inlines, .. } => out.push_str(&plain_text(inlines, depth + 1)),
            Inline::Math { latex } => out.push_str(latex),
            Inline::Image { alt, .. } => out.push_str(alt),
            Inline::Status { text, .. } => out.push_str(text),
            Inline::LineBreak => out.push(' '),
            Inline::FootnoteRef { .. } | Inline::AnchorMark { .. } => {}
        }
    }
    out
}
