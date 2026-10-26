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
//!   footnote definitions. One exception: in a sheet section, caption and note lines and the
//!   items of label and value pairs and of text regions end with their cell's address, for
//!   example `(A2)`; headings show none
//! - A sheet region with a layout starts with a line that says where it is and how it was
//!   found; a table region's header is its header paths, a grid shows the sheet's column
//!   letters and row numbers
//! - With [`TableStyle::Records`] a table region that is not unsure is one list item per
//!   row with the header path of each value; grids, pairs and text stay as they are
//! - A sheet selection ([`RenderOptions::sheets`]) writes only the chosen sheets' content,
//!   Logic parts and annotations, keeps the graph around them and every name and loss, and
//!   marks the other parts of the overview. The frontmatter names a chosen style and
//!   selection
//! - A hard line break is a backslash at the line end
//! - An annotation stands at its passage: after the first block with its anchor, or at the
//!   end of its page section. Only the others are listed in `## Annotations`. Blocks of a
//!   sheet take none: a cell comment stays in the list
//! - A body with fewer than 3 non-blank lines outside fences (Crystalline's Q001) gets the
//!   frontmatter description as a paragraph under the title. When it is still short, a last
//!   line says there is no content, or that it ends here when there is some (speaking of
//!   the chosen sections when the caller chose some)
//! - A leading level-1 heading equal to the title is not repeated; body headings shift down
//!   so none is level 1. Titled sections get `##`, nested ones `###` and so on
//! - A document whose body is over the part size is cut into units at its own borders by
//!   [`split`] (spec 5); the units' links are written for their place one folder down

mod blocks;
mod describe;
mod facets;
mod form;
mod frontmatter;
mod layout;
mod mermaid;
mod overview;
mod paging;
mod split;
mod workbook;

use std::collections::HashSet;

use rezzer_model::{Block, Completeness, Document, Inline, SectionKind, plain_text};

use crate::blocks::{Ctx, MAX_DEPTH, Notes};

pub use crate::split::{Unit, split};

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

/// How the rows of a sheet table are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableStyle {
    /// One Markdown table per table region.
    #[default]
    Table,
    /// One list item per data row, with the header path of each field.
    Records,
}

impl TableStyle {
    /// Every style, the default first.
    pub const ALL: [TableStyle; 2] = [TableStyle::Table, TableStyle::Records];

    /// The name callers use: `table` or `records`.
    pub fn name(self) -> &'static str {
        match self {
            TableStyle::Table => "table",
            TableStyle::Records => "records",
        }
    }
}

impl std::str::FromStr for TableStyle {
    type Err = String;

    /// Takes the [`TableStyle::name`]; case and surrounding spaces are allowed.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let wanted = s.trim().to_ascii_lowercase();
        TableStyle::ALL
            .into_iter()
            .find(|style| style.name() == wanted)
            .ok_or_else(|| {
                format!(
                    "unknown table style {}, expected table or records",
                    frontmatter::quote(s.trim())
                )
            })
    }
}

/// A sheet list in a message names at most this many sheets.
const MAX_NAMED_SHEETS: usize = 50;

/// The sheet titles that `selectors` name, in workbook order and without duplicates.
///
/// A selector is a sheet title, matched exactly, else without regard to case; else `#n`, the
/// n-th sheet in workbook order (1-based, hidden sheets counted). So a sheet named `#2` is
/// found by its name first. A selector that names nothing, or two sheets that differ only by
/// case, is an error that lists the sheets; so is any selector for a file without sheets.
pub fn resolve_sheets(doc: &Document, selectors: &[String]) -> Result<Vec<String>, String> {
    let titles: Vec<String> = doc
        .body
        .iter()
        .filter(|b| {
            matches!(
                b,
                Block::Section {
                    kind: SectionKind::Sheet,
                    ..
                }
            )
        })
        .map(|b| section_title(b).unwrap_or_default().to_string())
        .collect();
    if titles.is_empty() {
        return Err("this file has no sheets to select".to_string());
    }
    let mut chosen = vec![false; titles.len()];
    for selector in selectors {
        let exact: Vec<usize> = (0..titles.len())
            .filter(|&i| titles[i] == *selector)
            .collect();
        let found = if !exact.is_empty() {
            exact
        } else {
            let lower = selector.to_lowercase();
            let any_case: Vec<usize> = (0..titles.len())
                .filter(|&i| titles[i].to_lowercase() == lower)
                .collect();
            match any_case.len() {
                0 => position(selector, titles.len())
                    .map(|i| vec![i])
                    .ok_or_else(|| {
                        format!(
                            "unknown sheet {}; the workbook has these sheets: {}",
                            frontmatter::quote(selector),
                            sheet_list(&titles)
                        )
                    })?,
                1 => any_case,
                _ => {
                    let names: Vec<String> = any_case.iter().map(|&i| titles[i].clone()).collect();
                    return Err(format!(
                        "sheet {} matches more than one sheet: {}",
                        frontmatter::quote(selector),
                        sheet_list(&names)
                    ));
                }
            }
        };
        for i in found {
            chosen[i] = true;
        }
    }
    Ok(titles
        .into_iter()
        .zip(chosen)
        .filter_map(|(title, on)| on.then_some(title))
        .collect())
}

/// The 0-based index that a `#n` selector names, when n is a sheet's position.
fn position(selector: &str, count: usize) -> Option<usize> {
    let digits = selector.strip_prefix('#')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: usize = digits.parse().ok()?;
    (1..=count).contains(&n).then(|| n - 1)
}

/// The names quoted (only `"`, `\` and controls escaped) and joined by commas, at most 50,
/// then `and <n> more`.
fn sheet_list(names: &[String]) -> String {
    let shown: Vec<String> = names
        .iter()
        .take(MAX_NAMED_SHEETS)
        .map(|n| frontmatter::quote(n))
        .collect();
    let mut out = shown.join(", ");
    if names.len() > MAX_NAMED_SHEETS {
        out.push_str(&format!(" and {} more", names.len() - MAX_NAMED_SHEETS));
    }
    out
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
    /// How the rows of sheet tables are written. Default [`TableStyle::Table`].
    pub table_style: TableStyle,
    /// The titles of the sheets to write (see [`resolve_sheets`]); `None` or an empty list
    /// writes all.
    pub sheets: Option<Vec<String>>,
    /// Set when this output is one part of a split document: its place, location and title.
    pub part: Option<PartMeta>,
    /// Set on the main document of a split document: how many parts it has.
    pub parts: Option<usize>,
}

/// Where a part file stands in a split document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartMeta {
    /// The part's number, counted from 1.
    pub index: usize,
    /// How many parts the document has.
    pub count: usize,
    /// Where the part lies in the document, for example `pages 12 to 20` or `Data!A1:F400`.
    pub location: String,
    /// The part's own title, for example `Report: Results`.
    pub title: String,
}

impl Default for RenderOptions {
    fn default() -> RenderOptions {
        RenderOptions {
            sections: None,
            generated_at: String::new(),
            rezzer_version: String::new(),
            archive_ref: None,
            math: true,
            table_style: TableStyle::Table,
            sheets: None,
            part: None,
            parts: None,
        }
    }
}

/// Whether the tables of a document name a column without a header by its number: CSV and
/// TSV have no column letters of their own.
pub(crate) fn column_numbers(doc: &Document) -> bool {
    matches!(
        doc.source.format,
        rezzer_model::FormatId::Csv | rezzer_model::FormatId::Tsv
    )
}

/// Renders a document as OKF Markdown.
pub fn render(doc: &Document, completeness: Completeness, opts: &RenderOptions) -> String {
    let want = |s: OutputSection| opts.sections.as_ref().is_none_or(|l| l.contains(&s));
    let mut out = frontmatter::render(doc, completeness, opts);
    // A part has its own title; a leading heading equal to it is the title line.
    let shown = frontmatter::shown_title(doc, opts).trim();
    // The heading that became the title keeps its links.
    let title_line = match doc.body.first() {
        Some(Block::Heading {
            level: 1, inlines, ..
        }) if plain_text(inlines).trim() == shown => blocks::inlines(inlines, blocks::Mode::Line)
            .trim()
            .to_string(),
        _ => blocks::line_text(shown),
    };
    let heading = format!("\n{}\n", blocks::heading_line(1, &title_line));

    let selection = chosen_sheets(opts).map(|titles| Selection::of(doc, titles));
    let selection = selection.as_ref();
    let mut parts: Vec<String> = Vec::new();
    if want(OutputSection::Overview)
        && let Some(overview) = overview::render(doc, selection)
    {
        parts.push(overview);
    }
    if want(OutputSection::Properties) {
        parts.extend(facets::properties(doc));
    }
    let notes = Notes::new(&doc.facets.annotations);
    if want(OutputSection::Content) {
        parts.push(content(doc, shown, &notes, opts.table_style, selection));
    }
    if want(OutputSection::Logic) {
        parts.extend(facets::logic(doc, opts.math, selection));
    }
    let ctx = Ctx {
        codes_shown: Default::default(),
        assets: &doc.assets,
        link_base: "",
        heading_offset: 0,
        notes: None,
        in_sheet: Default::default(),
        table_style: opts.table_style,
        column_numbers: column_numbers(doc),
    };
    if want(OutputSection::Annotations) {
        parts.extend(facets::annotations(doc, &ctx, &notes, selection));
    }
    if want(OutputSection::Changes) {
        parts.extend(facets::changes(doc));
    }
    if want(OutputSection::Embedded) {
        parts.extend(facets::embedded(doc, ""));
    }
    if want(OutputSection::NotRead) {
        parts.extend(facets::not_read(doc));
    }
    // Crystalline wants 3 lines of content (Q001): a shorter body gets the description as
    // a paragraph under the title, and a body that is still short says it has nothing more.
    let content = parts
        .iter()
        .map(|p| describe::content_lines(p))
        .sum::<usize>();
    let lines = describe::content_lines(&heading) + content;
    if lines < MIN_CONTENT_LINES {
        let description = blocks::paragraph(&[Inline::Text {
            text: frontmatter::shown_description(doc, opts),
            emphasis: false,
            strong: false,
            strike: false,
        }]);
        if lines + describe::content_lines(&description) < MIN_CONTENT_LINES {
            parts.push(closing_line(content, opts.sections.is_some()).to_string());
        }
        parts.insert(0, description);
    }
    // One growth for all parts, so a huge part is not doubled by the push.
    out.reserve_exact(
        heading.len()
            + parts
                .iter()
                .map(|p| p.len() + if p.is_empty() { 0 } else { 2 })
                .sum::<usize>(),
    );
    out.push_str(&heading);
    for part in parts.iter().filter(|p| !p.is_empty()) {
        out.push('\n');
        out.push_str(part);
        out.push('\n');
    }
    out
}

/// Crystalline's least number of content lines in a body (its rule Q001).
const MIN_CONTENT_LINES: usize = 3;

/// The last line of a body still under [`MIN_CONTENT_LINES`] with its description: it says
/// there is no content only when there is none (`content` counts the lines under the title).
/// With chosen sections it speaks of them, as the document may hold more in others.
fn closing_line(content: usize, sections_chosen: bool) -> &'static str {
    match (content, sections_chosen) {
        (0, false) => "The document has no content.",
        (0, true) => "The chosen sections have no content.",
        (_, false) => "The document ends here.",
        (_, true) => "The chosen sections end here.",
    }
}

/// `## Overview`: the parts with kind, size, role and what they read from, the facts, and a
/// Mermaid graph of the parts when one reads from another. `None` when the document has no
/// overview (the enrich pass `overview` did not run) or it is empty.
pub fn render_overview(doc: &Document) -> Option<String> {
    overview::render(doc, None)
}

/// The sheets a render writes, when the caller chose some: what the content, the Logic
/// section, the annotations and the overview keep.
pub(crate) struct Selection<'a> {
    titles: HashSet<&'a str>,
    /// The anchors of the chosen sheets, as references write them (`'My sheet'`).
    chosen: HashSet<&'a str>,
    /// The anchors of every sheet.
    sheets: HashSet<&'a str>,
}

impl<'a> Selection<'a> {
    pub(crate) fn of(doc: &'a Document, titles: &'a [String]) -> Selection<'a> {
        let titles: HashSet<&str> = titles.iter().map(String::as_str).collect();
        let mut chosen = HashSet::new();
        let mut sheets = HashSet::new();
        for b in &doc.body {
            if let Block::Section {
                kind: SectionKind::Sheet,
                anchor: Some(anchor),
                ..
            } = b
            {
                sheets.insert(anchor.0.as_str());
                if section_title(b).is_some_and(|t| titles.contains(t)) {
                    chosen.insert(anchor.0.as_str());
                }
            }
        }
        Selection {
            titles,
            chosen,
            sheets,
        }
    }

    /// False for a sheet section that is not chosen; true for every other block.
    pub(crate) fn keeps_block(&self, b: &Block) -> bool {
        match b {
            Block::Section {
                kind: SectionKind::Sheet,
                ..
            } => section_title(b).is_some_and(|t| self.titles.contains(t)),
            _ => true,
        }
    }

    /// True for a sheet anchor (`Data`, `'My sheet'`) of a chosen sheet.
    pub(crate) fn chosen(&self, sheet: &str) -> bool {
        self.chosen.contains(sheet)
    }

    /// True for the anchor of a sheet that is not chosen; false for a chosen sheet and for
    /// anything that is no sheet's anchor (another part, a sheet without an anchor).
    pub(crate) fn left_out(&self, sheet: &str) -> bool {
        self.sheets.contains(sheet) && !self.chosen.contains(sheet)
    }

    /// False for an anchor on a sheet that is not chosen (`Data!A1`, or the sheet's own
    /// anchor); true for anything else, an anchor on no sheet included.
    pub(crate) fn keeps_anchor(&self, anchor: &str) -> bool {
        let sheet = workbook::split_sheet(anchor).map_or(anchor, |(sheet, _)| sheet);
        !self.sheets.contains(sheet) || self.chosen.contains(sheet)
    }
}

/// The chosen sheet titles; `None` when every sheet is written (no list, or an empty one).
pub(crate) fn chosen_sheets(opts: &RenderOptions) -> Option<&[String]> {
    opts.sheets.as_deref().filter(|titles| !titles.is_empty())
}

/// A sheet section's title as the selection names it: its title, else its anchor.
fn section_title(b: &Block) -> Option<&str> {
    match b {
        Block::Section { title, anchor, .. } => title
            .as_deref()
            .or_else(|| anchor.as_ref().map(|a| a.0.as_str())),
        _ => None,
    }
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

fn content(
    doc: &Document,
    title: &str,
    notes: &Notes,
    table_style: TableStyle,
    selection: Option<&Selection>,
) -> String {
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
        link_base: "",
        heading_offset: 2u8.saturating_sub(min),
        notes: Some(notes),
        in_sheet: Default::default(),
        table_style,
        column_numbers: column_numbers(doc),
    };
    let Some(selection) = selection else {
        return blocks::blocks(body, &ctx, 0, 2, false);
    };
    // The runs of blocks that stay, each rendered as one sequence.
    let mut parts: Vec<String> = Vec::new();
    for run in body
        .split(|b| !selection.keeps_block(b))
        .filter(|run| !run.is_empty())
    {
        let text = blocks::blocks(run, &ctx, 0, 2, false);
        if !text.is_empty() {
            parts.push(text);
        }
    }
    parts.join("\n\n")
}

/// The smallest heading level anywhere in `list`.
pub(crate) fn min_heading(list: &[Block], depth: usize) -> Option<u8> {
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

#[cfg(test)]
mod tests {
    use super::closing_line;

    #[test]
    fn closing_line_says_no_content_only_when_there_is_none() {
        assert_eq!(closing_line(0, false), "The document has no content.");
        assert_eq!(
            closing_line(0, true),
            "The chosen sections have no content."
        );
        assert_eq!(closing_line(1, false), "The document ends here.");
        assert_eq!(closing_line(1, true), "The chosen sections end here.");
    }
}
