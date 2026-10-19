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

use crate::anchors::{collect, sheet_prefix};
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
            .filter(|b| matches!(b, Block::Section { anchor, .. } if !is_footnote(anchor.as_ref().map(|a| a.0.as_str()))))
            .collect();
        let mut parts: Vec<OverviewPart> = Vec::new();
        let mut owner: HashMap<&str, usize> = HashMap::new();
        let mut by_name: HashMap<String, usize> = HashMap::new();
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
            by_name.entry(name.clone()).or_insert(i);
            parts.push(OverviewPart {
                name,
                kind: kind_words(*kind).to_string(),
                size: size(section),
                role: None,
                reads: Vec::new(),
            });
        }

        let part_of = |reference: &str| -> Option<usize> {
            owner.get(reference).copied().or_else(|| {
                let sheet = sheet_prefix(reference)?;
                owner
                    .get(sheet.as_str())
                    .or_else(|| by_name.get(&sheet))
                    .copied()
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
        let names: Vec<String> = parts.iter().map(|p| p.name.clone()).collect();
        for (part, set) in parts.iter_mut().zip(&reads) {
            part.reads = set.iter().map(|&q| names[q].clone()).collect();
        }

        doc.overview = Some(Model {
            parts,
            facts: facts(doc),
        });
    }
}

fn is_footnote(anchor: Option<&str>) -> bool {
    anchor.is_some_and(|a| a.starts_with("fn:"))
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

/// `<n> rows x <m> columns` for a sheet that starts with a table, else the paragraph count.
fn size(section: &Block) -> String {
    let Block::Section { kind, blocks, .. } = section else {
        return String::new();
    };
    if *kind == SectionKind::Sheet
        && let Some(Block::Table { rows, .. }) = blocks.first()
    {
        let columns = rows
            .iter()
            .map(|r| r.iter().map(|c| c.col_span.max(1) as usize).sum::<usize>())
            .max()
            .unwrap_or(0);
        return format!(
            "{} x {}",
            plural(rows.len(), "row", "rows"),
            plural(columns, "column", "columns")
        );
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
            .filter(|g| g.kind == "link")
            .map(|g| g.edges.len())
            .sum(),
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
