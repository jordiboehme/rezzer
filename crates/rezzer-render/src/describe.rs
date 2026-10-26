// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the frontmatter says about a document as a whole, for an import into Crystalline:
//! the tags, the description and the counts behind it, and the size checks Crystalline
//! makes on a body.
//!
//! Nothing here is guessed. A tag comes from the format, the kind of document, its own
//! keywords or the file system tags; a description is the document's own or a fixed
//! template of counted facts.

use std::collections::HashSet;

use rezzer_model::{Block, Document, FormatId, FormulaKind, LossReason, SectionKind};

use crate::blocks::MAX_DEPTH;
use crate::frontmatter::{document_date, format_name};

/// The tags of a document, in this order: `rezzer`, the format word, the kind words, the
/// folded document keywords, the folded file system tags. Each tag once, at its first place.
pub(crate) fn tags(doc: &Document) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut add = |tag: String| {
        if seen.insert(tag.clone()) {
            out.push(tag);
        }
    };
    add("rezzer".to_string());
    if let Some(format) = fold_tag(&format_name(doc.source.format)) {
        add(format);
    }
    for word in kind_words(doc) {
        add(word.to_string());
    }
    let raw = doc.properties.keywords.iter().chain(&doc.source.file_tags);
    for tag in raw.filter_map(|k| fold_tag(k)) {
        add(tag);
    }
    out
}

/// A tag as Crystalline takes it: lowercase, every run of whitespace, `_` and `-` one `-`,
/// no `-` at either end. `None` when the result is empty or holds anything but ASCII
/// letters, digits and `-`.
pub(crate) fn fold_tag(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    let mut gap = false;
    for c in raw.chars().flat_map(char::to_lowercase) {
        if c.is_whitespace() || c == '_' || c == '-' {
            gap = true;
            continue;
        }
        if !(c.is_ascii_lowercase() || c.is_ascii_digit()) {
            return None;
        }
        if gap && !out.is_empty() {
            out.push('-');
        }
        gap = false;
        out.push(c);
    }
    (!out.is_empty()).then_some(out)
}

/// The kinds of document its structure shows: `form` when it has form fields, `signed`
/// when a signature signs it (a usage rights signature alone does not).
pub(crate) fn kind_words(doc: &Document) -> Vec<&'static str> {
    let facts = &doc.properties.facts;
    let mut out = Vec::new();
    if !doc.facets.fields.is_empty() || facts.form_fields.is_some_and(|n| n > 0) {
        out.push("form");
    }
    let signed = match facts.signed {
        Some(signed) => signed,
        None => !doc.facets.signatures.is_empty(),
    };
    if signed {
        out.push("signed");
    }
    out
}

/// The counts a description and the main document's stats name. They follow the rules of
/// the `## Overview` facts, so both say the same.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    /// Tables without a role (a reader's summary has one); in a sheet only the table
    /// regions, not grids, label and value pairs or text.
    pub(crate) tables: usize,
    /// Outline sections (the PDF outline).
    pub(crate) outlines: usize,
    /// Formulas without the defined names and lambdas.
    pub(crate) formulas: usize,
    pub(crate) form_fields: usize,
    pub(crate) signatures: usize,
    pub(crate) annotations: usize,
    /// Pictures (assets) that need a look by the calling agent.
    pub(crate) vision: usize,
    /// Losses that say something was not read: every loss but an uncertain one.
    pub(crate) not_read: usize,
}

pub(crate) fn counts(doc: &Document) -> Counts {
    let mut tables = 0;
    let mut outlines = 0;
    count_blocks(&doc.body, 0, &mut tables, &mut outlines);
    Counts {
        tables,
        outlines,
        formulas: doc
            .facets
            .formulas
            .iter()
            .filter(|f| {
                !(matches!(f.kind, Some(FormulaKind::Name | FormulaKind::Lambda))
                    || f.anchor.0.starts_with("name:"))
            })
            .count(),
        form_fields: doc.facets.fields.len(),
        signatures: doc.facets.signatures.len(),
        annotations: doc.facets.annotations.len(),
        vision: doc.assets.iter().filter(|a| a.needs_vision).count(),
        not_read: doc
            .losses
            .iter()
            .filter(|l| l.why != LossReason::Uncertain)
            .count(),
    }
}

fn count_blocks(list: &[Block], depth: usize, tables: &mut usize, outlines: &mut usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for b in list {
        match b {
            Block::Table { role, layout, .. } => {
                let table_region = layout
                    .as_ref()
                    .is_none_or(|l| l.kind == rezzer_model::LayoutKind::Table);
                if role.is_none() && table_region {
                    *tables += 1;
                }
            }
            Block::Section {
                kind,
                title,
                blocks,
                ..
            } => {
                if *kind == SectionKind::Other && title.as_deref() == Some("Outline") {
                    *outlines += 1;
                } else {
                    count_blocks(blocks, depth + 1, tables, outlines);
                }
            }
            Block::Quote { blocks, .. } | Block::Callout { blocks, .. } => {
                count_blocks(blocks, depth + 1, tables, outlines);
            }
            Block::List { items, .. } => {
                for item in items {
                    count_blocks(item, depth + 1, tables, outlines);
                }
            }
            _ => {}
        }
    }
}

/// The document's own description, trimmed, else the template of counted facts:
/// `<label>[, <size>][, "<title>"][, <document date>].[ <counts>.]`, for example
/// `PDF, 48 pages, "SP 800-53 Rev. 5", 2020-09-23. 26 tables, 1 outline.` Only the
/// document's own date is used, never a file system date.
pub(crate) fn description(doc: &Document) -> String {
    if let Some(own) = doc
        .properties
        .description
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        return own.to_string();
    }
    let mut head = vec![format_label(doc.source.format)];
    if let Some(size) = size(doc) {
        head.push(size);
    }
    if let Some(own) = doc
        .properties
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        head.push(format!("\"{own}\""));
    }
    if let Some(date) = document_date(doc) {
        head.push(date.to_string());
    }
    let mut out = format!("{}.", head.join(", "));
    let c = counts(doc);
    let named: Vec<String> = [
        (c.tables, "table", "tables"),
        (c.outlines, "outline", "outlines"),
        (c.formulas, "formula", "formulas"),
        (c.form_fields, "form field", "form fields"),
        (c.signatures, "signature", "signatures"),
        (c.annotations, "annotation", "annotations"),
        (
            c.vision,
            "picture that needs vision",
            "pictures that need vision",
        ),
        (c.not_read, "part not read", "parts not read"),
    ]
    .into_iter()
    .filter(|(n, _, _)| *n > 0)
    .map(|(n, one, many)| counted(n, one, many))
    .collect();
    if !named.is_empty() {
        out.push_str(&format!(" {}.", named.join(", ")));
    }
    out
}

/// `1 page`, `2 pages`.
fn counted(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The size in the template: the pages of a PDF, the sheets of a workbook, the size of the
/// one part the overview has, else the bytes.
fn size(doc: &Document) -> Option<String> {
    if let Some(pages) = doc.properties.facts.pages {
        return Some(counted(pages as usize, "page", "pages"));
    }
    let workbook = matches!(
        doc.source.format,
        FormatId::Xlsx | FormatId::Xlsm | FormatId::Xlsb | FormatId::Ods | FormatId::Xls
    );
    let sheets = doc
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
        .count();
    if workbook && sheets > 0 {
        return Some(counted(sheets, "sheet", "sheets"));
    }
    if let Some(overview) = &doc.overview
        && let [one] = overview.parts.as_slice()
        && !one.size.trim().is_empty()
    {
        return Some(one.size.trim().to_string());
    }
    let bytes = usize::try_from(doc.source.size).unwrap_or(usize::MAX);
    Some(counted(bytes, "byte", "bytes"))
}

/// The name of a format in a description: `PDF`, `XLSX`, `Markdown`, `Text` and so on, else
/// the format name in upper case.
pub(crate) fn format_label(format: FormatId) -> String {
    match format {
        FormatId::Pdf => "PDF".to_string(),
        FormatId::Xlsx => "XLSX".to_string(),
        FormatId::Xlsm => "XLSM".to_string(),
        FormatId::Xlsb => "XLSB".to_string(),
        FormatId::Ods => "ODS".to_string(),
        FormatId::Csv => "CSV".to_string(),
        FormatId::Markdown => "Markdown".to_string(),
        FormatId::Text => "Text".to_string(),
        other => format_name(other).to_ascii_uppercase(),
    }
}

/// The token count Crystalline's size check (Q002) uses: the characters of the body after
/// the closing `---` line of the frontmatter, divided by four. Without frontmatter the whole
/// text is the body.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the splitter measures its parts with it")
)]
pub(crate) fn body_tokens(markdown: &str) -> usize {
    crate::paging::tokens(body(markdown))
}

/// The body of a Markdown file: what follows the closing `---` line of its frontmatter, the
/// whole text when it has none (no `---` first line, or no closing line).
fn body(markdown: &str) -> &str {
    let mut lines = markdown.split_inclusive('\n');
    let is_rule = |line: &str| line.trim_end_matches(['\n', '\r']) == "---";
    match lines.next() {
        Some(first) if first.ends_with('\n') && is_rule(first) => {}
        _ => return markdown,
    }
    let mut at = markdown.find('\n').map_or(markdown.len(), |i| i + 1);
    for line in lines {
        at += line.len();
        if is_rule(line) {
            return &markdown[at..];
        }
    }
    markdown
}

/// The lines Crystalline's content check (Q001) counts: non-blank lines outside fenced
/// code. The fence lines themselves count as inside, and an open fence runs to the end.
pub(crate) fn content_lines(body: &str) -> usize {
    let mut fence: Option<(char, usize)> = None;
    let mut n = 0;
    for line in body.split('\n') {
        let line = line.trim_end_matches('\r');
        let marker = crate::paging::fence_run(line);
        match fence {
            None => match marker {
                Some((ch, run, _)) => fence = Some((ch, run)),
                None if !line.trim().is_empty() => n += 1,
                None => {}
            },
            Some((ch, run)) => {
                if let Some((c, r, rest)) = marker
                    && c == ch
                    && r >= run
                    && rest.trim().is_empty()
                {
                    fence = None;
                }
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_tag_folds_and_drops() {
        assert_eq!(
            fold_tag("Security Controls").as_deref(),
            Some("security-controls")
        );
        assert_eq!(fold_tag("NIST_800").as_deref(), Some("nist-800"));
        assert_eq!(fold_tag(" --a _- b-- ").as_deref(), Some("a-b"));
        assert_eq!(fold_tag("Work\t\n Items").as_deref(), Some("work-items"));
        assert_eq!(fold_tag("Größe"), None);
        assert_eq!(fold_tag("c++"), None);
        assert_eq!(fold_tag("a.b"), None);
        assert_eq!(fold_tag(" - _ "), None);
        assert_eq!(fold_tag(""), None);
        // Lowercase first: a capital letter is never a reason to drop a tag.
        assert_eq!(fold_tag("ABC9").as_deref(), Some("abc9"));
    }

    #[test]
    fn format_labels() {
        assert_eq!(format_label(FormatId::Pdf), "PDF");
        assert_eq!(format_label(FormatId::Xlsb), "XLSB");
        assert_eq!(format_label(FormatId::Markdown), "Markdown");
        assert_eq!(format_label(FormatId::Text), "Text");
        assert_eq!(format_label(FormatId::ThreeMf), "THREE_MF");
        assert_eq!(format_label(FormatId::Docx), "DOCX");
    }

    #[test]
    fn body_tokens_count_after_the_closing_line() {
        // 8 characters of body: "\n# T\nabc" is 8 chars, 2 tokens.
        assert_eq!(body_tokens("---\ntype: source\n---\n\n# T\nabc"), 2);
        assert_eq!(
            body(&format!("---\na: b\n---\n{}", "x".repeat(40_003))).len(),
            40_003
        );
        assert_eq!(
            body_tokens(&format!("---\na: b\n---\n{}", "x".repeat(40_003))),
            10_000
        );
        assert_eq!(
            body_tokens(&format!("---\na: b\n---\n{}", "é".repeat(40_004))),
            10_001
        );
        assert_eq!(body("---\r\na: b\r\n---\r\nbody"), "body");
        assert_eq!(body("---\n---\nbody"), "body");
        // No frontmatter, or one that never closes: all is body.
        assert_eq!(body("# T\n---\nx"), "# T\n---\nx");
        assert_eq!(body("---\na: b\nno end"), "---\na: b\nno end");
        assert_eq!(body("---"), "---");
    }

    #[test]
    fn content_lines_skip_fences_and_blanks() {
        assert_eq!(content_lines("\n# T\n\nText\n"), 2);
        assert_eq!(content_lines("a\n```\nb\nc\n```\nd"), 2);
        // A shorter or different closer does not close; text after the run does not either.
        assert_eq!(content_lines("````\na\n```\n~~~~\n```` x\nb\n````\nc"), 1);
        // An open fence runs to the end; a deep indent is no fence.
        assert_eq!(content_lines("a\n~~~\nb\nc"), 1);
        assert_eq!(content_lines("a\n    ```\nb"), 3);
    }
}
