// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Pass `overview`: builds `doc.overview` from the body and the facets.
//!
//! Parts are the top-level sections. Footnote sections (anchor `fn:...`) are definitions of
//! the text, not parts, so they are left out. A formula
//! belongs to the part that holds its anchor; for a spreadsheet reference such as
//! `Costs!A1:B9` it is the sheet part. A part with formulas is a `result`; a part that other
//! parts read from and that has no formulas is an `input`.

use std::collections::{BTreeSet, HashMap};

use rezzer_model::{
    Block, Document, Inline, Overview as Model, OverviewPart, Role, SectionKind, Value,
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
            .filter(|b| !is_footnote_section(b))
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
/// When the document has only this one sheet and the reader gave `rows` and `columns`
/// properties, those are the size and the rows in the table are noted as shown when fewer.
/// For any other part: the paragraph count.
fn size(section: &Block, totals: Option<(usize, usize)>) -> String {
    let Block::Section { kind, blocks, .. } = section else {
        return String::new();
    };
    if *kind == SectionKind::Sheet {
        let table = blocks.iter().find_map(|b| match b {
            Block::Table {
                rows,
                header_rows,
                role: None,
                ..
            } => Some((rows, *header_rows)),
            _ => None,
        });
        let shown = table.map(|(rows, header)| {
            let columns = rows
                .iter()
                .map(|r| r.iter().map(|c| c.col_span.max(1) as usize).sum::<usize>())
                .max()
                .unwrap_or(0);
            (rows.len().saturating_sub(header), columns)
        });
        match (totals, shown) {
            (Some((rows, columns)), shown) => {
                let base = format!(
                    "{} x {}",
                    plural(rows, "row", "rows"),
                    plural(columns, "column", "columns")
                );
                return match shown {
                    Some((n, _)) if n >= rows => base,
                    Some((n, _)) => format!("{base} ({n} shown)"),
                    None => format!("{base} (none shown)"),
                };
            }
            (None, Some((rows, columns))) => {
                return format!(
                    "{} x {}",
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

/// The facts in display order; counts that are zero are left out.
fn facts(doc: &Document) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut count = |label: &str, n: usize| {
        if n > 0 {
            out.push((label.to_string(), n.to_string()));
        }
    };
    count("Formulas", doc.facets.formulas.len());
    count("Graphs", doc.facets.graphs.len());
    count(
        "Links",
        doc.facets
            .graphs
            .iter()
            .flat_map(|g| &g.edges)
            .filter(|e| e.kind == "links_to")
            .count(),
    );
    count("Images", images(doc));
    count("Annotations", doc.facets.annotations.len());
    count("Changes", doc.facets.timeline.len());
    count("Embedded", doc.facets.embedded.len());
    count("Not read", doc.losses.len());
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

/// Media blocks whose asset is an image, plus image inlines in the body.
fn images(doc: &Document) -> usize {
    let mime: HashMap<&str, &str> = doc
        .assets
        .iter()
        .map(|a| (a.id.as_str(), a.mime.as_str()))
        .collect();
    let mut n = 0;
    walk::blocks(&doc.body, |b| {
        if let Block::Media { asset, .. } = b
            && mime
                .get(asset.as_str())
                .is_some_and(|m| m.starts_with("image/"))
        {
            n += 1;
        }
        walk::block_inlines(b, |list| {
            walk::inlines(list, |i| {
                if matches!(i, Inline::Image { .. }) {
                    n += 1;
                }
            });
        });
        true
    });
    n
}
