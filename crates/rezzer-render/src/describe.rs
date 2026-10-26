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

use rezzer_model::{Block, Document, FormatId, LayoutKind, SectionKind, Signature};

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
        // The reader could not say: a signature in the list that is not a usage rights one.
        None => doc
            .facets
            .signatures
            .iter()
            .any(|s| s.kind != Signature::USAGE_RIGHTS),
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
    /// 1 when the document keeps an outline (the PDF bookmarks), else 0.
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
    let mut counts = Counts {
        formulas: doc.facets.formulas.iter().filter(|f| !f.is_name()).count(),
        form_fields: doc.facets.fields.len(),
        signatures: doc.facets.signatures.len(),
        annotations: doc.facets.annotations.len(),
        outlines: usize::from(!doc.facets.outline.is_empty()),
        vision: doc.assets.iter().filter(|a| a.needs_vision).count(),
        not_read: doc.losses.iter().filter(|l| l.is_not_read()).count(),
        ..Counts::default()
    };
    count_blocks(&doc.body, 0, &mut counts);
    counts
}

/// Adds the tables in `list` to `counts`.
fn count_blocks(list: &[Block], depth: usize, counts: &mut Counts) {
    if depth > MAX_DEPTH {
        return;
    }
    for b in list {
        match b {
            Block::Table { role, layout, .. } => {
                let table_region = layout.as_ref().is_none_or(|l| l.kind == LayoutKind::Table);
                if role.is_none() && table_region {
                    counts.tables += 1;
                }
            }
            Block::Section { blocks, .. }
            | Block::Quote { blocks, .. }
            | Block::Callout { blocks, .. } => {
                count_blocks(blocks, depth + 1, counts);
            }
            Block::List { items, .. } => {
                for item in items {
                    count_blocks(item, depth + 1, counts);
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
    let mut head = vec![format_label(doc.source.format), size(doc)];
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
fn size(doc: &Document) -> String {
    if let Some(pages) = doc.properties.facts.pages {
        return counted(pages as usize, "page", "pages");
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
        return counted(sheets, "sheet", "sheets");
    }
    if let Some(overview) = &doc.overview
        && let [one] = overview.parts.as_slice()
        && !one.size.trim().is_empty()
    {
        return one.size.trim().to_string();
    }
    let bytes = usize::try_from(doc.source.size).unwrap_or(usize::MAX);
    counted(bytes, "byte", "bytes")
}

/// The name of a format in a description, as people write it: `PDF`, `Markdown`, `3MF`,
/// `KiCad schematic`, `Empty file`. A format without a label of its own (one added later)
/// is its name with spaces in upper case.
pub(crate) fn format_label(format: FormatId) -> String {
    let label = match format {
        FormatId::Empty => "Empty file",
        FormatId::Text => "Text",
        FormatId::Markdown => "Markdown",
        FormatId::Csv => "CSV",
        FormatId::Tsv => "TSV",
        FormatId::Html => "HTML",
        FormatId::Epub => "EPUB",
        FormatId::Pdf => "PDF",
        FormatId::PostScript => "PostScript",
        FormatId::Eps => "EPS",
        FormatId::Docx => "DOCX",
        FormatId::Xlsx => "XLSX",
        FormatId::Xlsm => "XLSM",
        FormatId::Xlsb => "XLSB",
        FormatId::Pptx => "PPTX",
        FormatId::Odt => "ODT",
        FormatId::Ods => "ODS",
        FormatId::Odp => "ODP",
        FormatId::Doc => "DOC",
        FormatId::Xls => "XLS",
        FormatId::Ppt => "PPT",
        FormatId::Rtf => "RTF",
        FormatId::Eml => "Email",
        FormatId::Mbox => "Mailbox",
        FormatId::Zip => "ZIP",
        FormatId::Fcstd => "FreeCAD",
        FormatId::Step => "STEP",
        FormatId::Iges => "IGES",
        FormatId::Ifc => "IFC",
        FormatId::Dxf => "DXF",
        FormatId::Dwg => "DWG",
        FormatId::ThreeMf => "3MF",
        FormatId::Stl => "STL",
        FormatId::Obj => "OBJ",
        FormatId::Gltf => "glTF",
        FormatId::Glb => "GLB",
        FormatId::Gcode => "G-code",
        FormatId::Bgcode => "Binary G-code",
        FormatId::KicadSch => "KiCad schematic",
        FormatId::KicadPcb => "KiCad board",
        FormatId::Gerber => "Gerber",
        FormatId::Excellon => "Excellon drill",
        FormatId::Openscad => "OpenSCAD",
        FormatId::Sldprt => "SolidWorks part",
        FormatId::CreoPrt => "Creo part",
        FormatId::NxPrt => "NX part",
        FormatId::Catpart => "CATIA part",
        FormatId::Ipt => "Inventor part",
        FormatId::Iam => "Inventor assembly",
        FormatId::F3d => "Fusion design",
        FormatId::Rhino3dm => "Rhino 3DM",
        FormatId::Ole => "OLE compound file",
        FormatId::Unknown => "Unknown format",
        other => return format_name(other).replace('_', " ").to_ascii_uppercase(),
    };
    label.to_string()
}

/// The token count Crystalline's size check (Q002) uses, as an import sees the file
/// (decision 12): the characters of the body after the closing `---` line of the frontmatter,
/// divided by four, each link to a file counted at its import length ([`Measure`]).
/// `note_path_len` is the length of the file's path without `.md`. Without frontmatter the
/// whole text is the body.
pub(crate) fn body_tokens(markdown: &str, note_path_len: usize) -> usize {
    Measure {
        note_path_len,
        moved: false,
    }
    .chars(body(markdown))
        / 4
}

/// The one size measure of Rezzer's output (decisions 12 and F11): characters, plus what the
/// links in the text can grow by.
///
/// - A link to a file (a relative destination that is no anchor and no `.md` file) counts at
///   the longer of its written length and the link Crystalline's import writes for it,
///   `assets/` + the note's path without `.md` + `/` + the file name.
/// - With `moved` the text is still to be written as a part one folder down: every relative
///   destination gets 3 characters for `../`, and every anchor link (`](#`) 64 characters
///   for the part file it may point to (at most 60 bytes plus `.md`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Measure {
    /// Characters of the note's path without `.md`, as the import root sees it.
    pub note_path_len: usize,
    pub moved: bool,
}

/// What an anchor link may grow by when its anchor is in another part: the part's file name
/// (at most 60 bytes) and `.md`, with room to spare.
pub(crate) const ANCHOR_LINK_GROWTH: usize = 64;

impl Measure {
    /// The size of `text` in characters, its links counted as above.
    pub(crate) fn chars(&self, text: &str) -> usize {
        text.chars().count() + self.growth(text)
    }

    /// What the links in `text` can grow by.
    fn growth(&self, text: &str) -> usize {
        let mut growth = 0usize;
        let mut rest = text;
        while let Some(at) = rest.find("](") {
            rest = &rest[at + 2..];
            let end = rest.find([')', ' ', '\n']).unwrap_or(rest.len());
            let dest = &rest[..end];
            if dest.starts_with('#') {
                if self.moved {
                    growth += ANCHOR_LINK_GROWTH;
                }
                continue;
            }
            if !relative(dest) {
                continue;
            }
            if self.moved {
                growth += 3;
            }
            let path = dest.split('#').next().unwrap_or(dest);
            if path.ends_with(".md") {
                continue;
            }
            let name = path.rsplit('/').next().unwrap_or(path);
            let import = "assets/".len() + self.note_path_len + 1 + name.chars().count();
            growth += import.saturating_sub(dest.chars().count());
        }
        growth
    }
}

/// A link destination that is a relative path: not empty, no anchor alone, no `/` first and
/// no scheme (`https:`, `mailto:`).
fn relative(dest: &str) -> bool {
    if dest.is_empty() || dest.starts_with(['#', '/', '<']) {
        return false;
    }
    let before = dest.split(['/', '?', '#']).next().unwrap_or("");
    !before.contains(':')
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
        assert_eq!(format_label(FormatId::ThreeMf), "3MF");
        assert_eq!(format_label(FormatId::Docx), "DOCX");
        assert_eq!(format_label(FormatId::Empty), "Empty file");
        assert_eq!(format_label(FormatId::Unknown), "Unknown format");
        assert_eq!(format_label(FormatId::PostScript), "PostScript");
        assert_eq!(format_label(FormatId::KicadSch), "KiCad schematic");
        assert_eq!(format_label(FormatId::CreoPrt), "Creo part");
        // The tag word stays the format name.
        assert_eq!(
            fold_tag(&format_name(FormatId::ThreeMf)).as_deref(),
            Some("three-mf")
        );
    }

    #[test]
    fn body_tokens_count_after_the_closing_line() {
        // 8 characters of body: "\n# T\nabc" is 8 chars, 2 tokens.
        assert_eq!(body_tokens("---\ntype: source\n---\n\n# T\nabc", 0), 2);
        assert_eq!(
            body(&format!("---\na: b\n---\n{}", "x".repeat(40_003))).len(),
            40_003
        );
        assert_eq!(
            body_tokens(&format!("---\na: b\n---\n{}", "x".repeat(40_003)), 0),
            10_000
        );
        assert_eq!(
            body_tokens(&format!("---\na: b\n---\n{}", "é".repeat(40_004)), 0),
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
    fn measure_counts_links_at_their_import_length() {
        use super::Measure;
        let plain = Measure {
            note_path_len: 10,
            moved: false,
        };
        // `assets/` + 10 + `/` + `a.png` is 23 characters, the written link 14.
        assert_eq!(plain.chars("![p](b.assets/a.png)"), 20 + 23 - 14);
        // Web addresses, anchors and notes do not grow.
        assert_eq!(plain.chars("[x](https://e.org/a.png)"), 24);
        assert_eq!(plain.chars("[x](#here)"), 10);
        assert_eq!(plain.chars("[x](other.md#a)"), 15);
        let moved = Measure {
            note_path_len: 10,
            moved: true,
        };
        assert_eq!(moved.chars("[x](#here)"), 10 + super::ANCHOR_LINK_GROWTH);
        assert_eq!(moved.chars("[x](other.md)"), 13 + 3);
        assert_eq!(
            body_tokens("---\na: b\n---\n![p](b.assets/a.png)", 10),
            (20 + 9) / 4
        );
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
