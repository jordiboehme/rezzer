// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Pass `overview`: builds `doc.overview` from the body and the facets.
//!
//! Parts are the top-level sections. Footnote sections (anchor `fn:...`) are definitions of
//! the text, not parts, so they are left out, and so are page sections: the pages of a PDF
//! are counted as a fact, as a list of pages says nothing a reader needs. A formula
//! belongs to the part that holds its anchor; for a spreadsheet reference such as
//! `Costs!A1:B9` it is the sheet part. A part with formulas is a `result`; a part that other
//! parts read from and that has no formulas is an `input`.

use std::collections::{BTreeSet, HashMap, HashSet};

use rezzer_model::{
    Block, Document, FormatId, FormulaKind, Inline, LayoutKind, LossReason, Overview as Model,
    OverviewPart, Role, SectionKind, Value,
};

use crate::anchors::{collect, sheet_prefixes};
use crate::{Pass, walk};

pub(crate) struct Overview;

impl Pass for Overview {
    fn name(&self) -> &'static str {
        "overview"
    }

    fn run(&self, doc: &mut Document) {
        let sections: Vec<&Block> = doc
            .body
            .iter()
            .filter(|b| !is_footnote_section(b) && !is_page(b))
            .collect();
        let mut parts: Vec<OverviewPart> = Vec::new();
        let mut owner: HashMap<&str, usize> = HashMap::new();
        let mut by_id: HashMap<String, usize> = HashMap::new();
        // The reader's row and column counts describe the sheet only when there is one.
        let sheets = sections
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
        let totals = if sheets == 1 {
            reader_totals(doc)
        } else {
            None
        };
        for (i, section) in sections.iter().enumerate() {
            let Block::Section {
                kind,
                title,
                anchor,
                ..
            } = section
            else {
                continue;
            };
            let name = title
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .or_else(|| anchor.as_ref().map(|a| a.0.clone()))
                .unwrap_or_else(|| format!("Part {}", i + 1));
            let mut found = Vec::new();
            let mut unused = Vec::new();
            collect(std::slice::from_ref(*section), &mut found, &mut unused);
            for a in found {
                owner.entry(a.0.as_str()).or_insert(i);
            }
            let id = match anchor {
                Some(a) if !by_id.contains_key(&a.0) => a.0.clone(),
                _ => format!("part:{}", i + 1),
            };
            by_id.insert(id.clone(), i);
            parts.push(OverviewPart {
                id,
                name,
                kind: kind_words(*kind).to_string(),
                size: size(section, totals),
                role: None,
                reads: Vec::new(),
            });
        }

        let part_of = |reference: &str| -> Option<usize> {
            owner.get(reference).copied().or_else(|| {
                sheet_prefixes(reference)
                    .iter()
                    .find_map(|sheet| owner.get(sheet.as_str()).copied())
            })
        };
        let mut has_formulas = vec![false; parts.len()];
        let mut read_by_others = vec![false; parts.len()];
        let mut reads: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); parts.len()];
        for f in &doc.facets.formulas {
            let Some(p) = part_of(&f.anchor.0) else {
                continue;
            };
            has_formulas[p] = true;
            for r in &f.references {
                if let Some(q) = part_of(&r.0)
                    && q != p
                {
                    reads[p].insert(q);
                    read_by_others[q] = true;
                }
            }
        }
        for (i, part) in parts.iter_mut().enumerate() {
            part.role = if has_formulas[i] {
                Some(Role::Result)
            } else if read_by_others[i] {
                Some(Role::Input)
            } else {
                None
            };
        }
        let ids: Vec<String> = parts.iter().map(|p| p.id.clone()).collect();
        for (part, set) in parts.iter_mut().zip(&reads) {
            part.reads = set.iter().map(|&q| ids[q].clone()).collect();
        }

        doc.overview = Some(Model {
            parts,
            facts: facts(doc),
        });
    }
}

fn is_page(block: &Block) -> bool {
    matches!(
        block,
        Block::Section {
            kind: SectionKind::Page,
            ..
        }
    )
}

fn is_footnote_anchor(block: &Block) -> bool {
    matches!(block, Block::Section { anchor: Some(a), .. } if a.0.starts_with("fn:"))
}

/// A footnote section, or the wrapper that holds only footnote sections (the Markdown
/// reader's "Footnotes").
fn is_footnote_section(block: &Block) -> bool {
    match block {
        Block::Section { blocks, .. } => {
            is_footnote_anchor(block)
                || (!blocks.is_empty() && blocks.iter().all(is_footnote_anchor))
        }
        _ => false,
    }
}

fn kind_words(kind: SectionKind) -> &'static str {
    match kind {
        SectionKind::Chapter => "chapter",
        SectionKind::Sheet => "sheet",
        SectionKind::Slide => "slide",
        SectionKind::Page => "page",
        SectionKind::Message => "message",
        SectionKind::Part => "part",
        SectionKind::Layer => "layer",
        SectionKind::Other => "other",
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// For a sheet: `<n> rows x <m> columns`, data rows without header rows, from the first table
/// without a role (a table a reader generated, such as a summary, is not the sheet's data).
/// When a layout split the sheet into regions, the first region of kind table counts, and
/// `, <k> regions` follows when there is more than one; a sheet without a table region is
/// `<k> regions`.
/// When the document has only this one sheet and the reader gave `rows` and `columns`
/// properties, those are the size and the rows in the table (with regions: in all region
/// tables, without their header rows) are noted as shown when fewer.
/// For any other part: the paragraph count.
fn size(section: &Block, totals: Option<(usize, usize)>) -> String {
    let Block::Section { kind, blocks, .. } = section else {
        return String::new();
    };
    if *kind == SectionKind::Sheet {
        // The sheet's regions, when a layout found them (decision 10): the first region of
        // kind table gives the size, and the count is added when there are more.
        let regions = blocks
            .iter()
            .filter(|b| {
                matches!(
                    b,
                    Block::Table {
                        layout: Some(_),
                        ..
                    }
                )
            })
            .count();
        let table = blocks.iter().find_map(|b| match b {
            Block::Table {
                rows,
                header_rows,
                role: None,
                layout,
                ..
            } if regions == 0 || layout.as_ref().is_some_and(|l| l.kind == LayoutKind::Table) => {
                Some((rows, *header_rows))
            }
            _ => None,
        });
        let more = match regions {
            0 | 1 => String::new(),
            n => format!(", {n} regions"),
        };
        if regions > 0 && table.is_none() && totals.is_none() {
            return plural(regions, "region", "regions");
        }
        let mut shown = table.map(|(rows, header)| {
            let columns = rows
                .iter()
                .map(|r| r.iter().map(|c| c.col_span.max(1) as usize).sum::<usize>())
                .max()
                .unwrap_or(0);
            (rows.len().saturating_sub(header), columns)
        });
        // Against the reader's totals, every region's rows count as shown.
        if totals.is_some() && regions > 0 {
            let rows = blocks
                .iter()
                .filter_map(|b| match b {
                    Block::Table {
                        rows,
                        header_rows,
                        layout: Some(_),
                        ..
                    } => Some(rows.len().saturating_sub(*header_rows)),
                    _ => None,
                })
                .sum();
            shown = Some((rows, 0));
        }
        match (totals, shown) {
            (Some((rows, columns)), shown) => {
                let base = format!(
                    "{} x {}",
                    plural(rows, "row", "rows"),
                    plural(columns, "column", "columns")
                );
                return match shown {
                    Some((n, _)) if n >= rows => format!("{base}{more}"),
                    Some((n, _)) => format!("{base} ({n} shown){more}"),
                    None => format!("{base} (none shown)"),
                };
            }
            (None, Some((rows, columns))) => {
                return format!(
                    "{} x {}{more}",
                    plural(rows, "row", "rows"),
                    plural(columns, "column", "columns")
                );
            }
            (None, None) => {}
        }
    }
    let mut paragraphs = 0;
    walk::blocks(blocks, |b| {
        if matches!(b, Block::Paragraph { .. }) {
            paragraphs += 1;
        }
        true
    });
    plural(paragraphs, "paragraph", "paragraphs")
}

/// The `rows` and `columns` counts a reader put in the properties, when both are whole numbers.
fn reader_totals(doc: &Document) -> Option<(usize, usize)> {
    let count = |key: &str| match doc.properties.custom.get(key) {
        Some(Value::Number { value, .. }) if *value >= 0.0 && value.fract() == 0.0 => {
            Some(*value as usize)
        }
        _ => None,
    };
    Some((count("rows")?, count("columns")?))
}

/// True for the workbook formats, whose facts count the spreadsheet logic.
fn is_workbook(doc: &Document) -> bool {
    matches!(
        doc.source.format,
        FormatId::Xlsx | FormatId::Xlsm | FormatId::Xlsb | FormatId::Ods | FormatId::Xls
    )
}

/// A sheet section whose first block is the reader's `hidden` or `very hidden` status.
fn is_hidden_sheet(block: &Block) -> bool {
    let Block::Section { blocks, .. } = block else {
        return false;
    };
    match blocks.first() {
        Some(Block::Paragraph { inlines, role, .. }) if role.as_deref() == Some("status") => {
            inlines.iter().any(|i| {
                matches!(i, Inline::Status { text, .. } if text == "hidden" || text == "very hidden")
            })
        }
        _ => false,
    }
}

/// The workbook counts: sheets (hidden ones named), formulas (cells, not names), patterns,
/// parameters, results, names, comments and external links.
fn workbook_facts(doc: &Document, out: &mut Vec<(String, String)>) {
    let sheets: Vec<&Block> = doc
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
        .collect();
    if !sheets.is_empty() {
        let hidden: Vec<String> = sheets
            .iter()
            .filter(|b| is_hidden_sheet(b))
            .filter_map(|b| match b {
                Block::Section { title, anchor, .. } => title
                    .clone()
                    .or_else(|| anchor.as_ref().map(|a| a.0.clone())),
                _ => None,
            })
            .collect();
        let value = if hidden.is_empty() {
            sheets.len().to_string()
        } else {
            format!("{} (hidden: {})", sheets.len(), hidden.join(", "))
        };
        out.push(("Sheets".into(), value));
    }
    let is_name = |f: &&rezzer_model::Formula| {
        matches!(f.kind, Some(FormulaKind::Name | FormulaKind::Lambda))
            || f.anchor.0.starts_with("name:")
    };
    let formulas = &doc.facets.formulas;
    let patterns: HashSet<&str> = formulas
        .iter()
        .filter_map(|f| f.pattern.as_deref())
        .collect();
    let mut count = |label: &str, n: usize| {
        if n > 0 {
            out.push((label.to_string(), n.to_string()));
        }
    };
    count("Formulas", formulas.iter().filter(|f| !is_name(f)).count());
    count("Patterns", patterns.len());
    count("Parameters", doc.facets.parameters.len());
    count(
        "Results",
        formulas
            .iter()
            .filter(|f| !is_name(f) && f.role == Some(Role::Result))
            .count(),
    );
    count("Names", formulas.iter().filter(is_name).count());
    count("Comments", doc.facets.annotations.len());
    count(
        "External links",
        doc.facets
            .embedded
            .iter()
            .filter(|e| e.kind == "external-workbook" || e.kind == "add-in")
            .count(),
    );
}

/// The facts in display order; counts that are zero are left out. A workbook gets its own
/// counts first and no graph count (its one graph is the dependency graph in Logic).
fn facts(doc: &Document) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let workbook = is_workbook(doc);
    if workbook {
        workbook_facts(doc, &mut out);
    }
    let mut count = |label: &str, n: usize| {
        if n > 0 {
            out.push((label.to_string(), n.to_string()));
        }
    };
    count("Pages", doc.body.iter().filter(|b| is_page(b)).count());
    if !workbook {
        count("Formulas", doc.facets.formulas.len());
        count("Graphs", doc.facets.graphs.len());
    }
    count(
        "Links",
        doc.facets
            .graphs
            .iter()
            .flat_map(|g| &g.edges)
            .filter(|e| e.kind == "links_to")
            .count(),
    );
    let (places, distinct) = images(doc);
    count("Images", places);
    // Barcodes and QR codes read, by kind; a picture that looks like one but gave none.
    let mut kinds: Vec<(&str, usize)> = Vec::new();
    for c in doc.assets.iter().flat_map(|a| &a.codes) {
        match kinds.iter_mut().find(|(k, _)| *k == c.symbology) {
            Some((_, n)) => *n += 1,
            None => kinds.push((&c.symbology, 1)),
        }
    }
    let unread = doc.assets.iter().filter(|a| a.code_unread).count();
    if !workbook {
        count("Annotations", doc.facets.annotations.len());
    }
    count("Form fields", doc.facets.fields.len());
    count(
        "Scripts",
        doc.facets
            .embedded
            .iter()
            .filter(|e| e.kind == "script")
            .count(),
    );
    count("Signatures", doc.facets.signatures.len());
    count("Changes", doc.facets.timeline.len());
    // Scripts have their own count.
    count(
        "Embedded",
        doc.facets
            .embedded
            .iter()
            .filter(|e| e.kind != "script")
            .count(),
    );
    // Losses that say something was not read: every loss but an uncertain one. An
    // uncertain loss (an unsure region, an overlap, a guessed dialect) stays listed with the
    // losses, but its content was read.
    count(
        "Not read",
        doc.losses
            .iter()
            .filter(|l| l.why != LossReason::Uncertain)
            .count(),
    );
    // The codes stand right after the pictures they were read from.
    let mut codes: Vec<(String, String)> = Vec::new();
    if !kinds.is_empty() {
        let total: usize = kinds.iter().map(|(_, n)| n).sum();
        let each: Vec<String> = kinds.iter().map(|(k, n)| format!("{k} {n}")).collect();
        codes.push(("Codes".into(), format!("{total} ({})", each.join(", "))));
    }
    if unread > 0 {
        codes.push(("Codes not decoded".into(), unread.to_string()));
    }
    let at = ["Images", "Links", "Graphs", "Formulas", "Pages"]
        .iter()
        .find_map(|k| out.iter().position(|(l, _)| l == k))
        .map_or(0, |i| i + 1);
    out.splice(at..at, codes);
    // One picture drawn at several places (a logo on every page) is one file.
    if distinct < places
        && let Some((_, v)) = out.iter_mut().find(|(k, _)| k == "Images")
    {
        *v = format!("{places} ({distinct} distinct)");
    }
    let changed = [&doc.properties.modified, &doc.properties.created]
        .into_iter()
        .flatten()
        .find_map(date_text);
    if let Some(changed) = changed {
        out.push(("Last changed".into(), changed));
    }
    out
}

fn date_text(v: &Value) -> Option<String> {
    match v {
        Value::Text { value } | Value::Date { value } | Value::DateTime { value } => {
            Some(value.trim().to_string()).filter(|s| !s.is_empty())
        }
        _ => None,
    }
}

/// Media blocks whose asset is an image, plus image inlines in the body: how many places,
/// and how many distinct pictures (assets, and image inlines by their target).
fn images(doc: &Document) -> (usize, usize) {
    let mime: HashMap<&str, &str> = doc
        .assets
        .iter()
        .map(|a| (a.id.as_str(), a.mime.as_str()))
        .collect();
    let mut n = 0;
    let mut seen: HashSet<String> = HashSet::new();
    walk::blocks(&doc.body, |b| {
        if let Block::Media { asset, .. } = b
            && mime
                .get(asset.as_str())
                .is_some_and(|m| m.starts_with("image/"))
        {
            n += 1;
            if !seen.contains(asset.as_str()) {
                seen.insert(asset.clone());
            }
        }
        walk::block_inlines(b, |list| {
            walk::inlines(list, |i| {
                if let Inline::Image { target, .. } = i {
                    n += 1;
                    if !seen.contains(target.as_str()) {
                        seen.insert(target.clone());
                    }
                }
            });
        });
        true
    });
    (n, seen.len())
}
