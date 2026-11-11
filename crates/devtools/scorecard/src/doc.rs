// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A document as the facts see it: its texts in reading order, one unit per block, list item
//! paragraph or table cell, with the lines of each; its tables as grids; its links and the
//! kind of the region each block stands in. Form fields listed beside the body are not in it. Words Rezzer adds itself (the page
//! marker, "Found by" lines, picture notes, status marks) are left out: a fact is about the
//! file.

use std::collections::HashMap;

use rezzer_model::{Block, Cell, Document, Inline, RegionKind, RegionNote, Value};

use crate::text::{Decimal, decimal_of, lines, norm};

/// Roles of paragraphs that hold Rezzer's own words.
const OWN_ROLES: [&str; 3] = ["page", "found-by", "pictures"];

/// The largest span of a cell that is followed; a larger one is cut to it.
const MAX_SPAN: usize = 1024;

/// One text of the document: a block, or a table cell.
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    /// Its lines, normalised; empty lines are kept so that a line keeps its number.
    pub lines: Vec<String>,
    /// Its lines that hold text, joined by spaces.
    pub text: String,
    /// The anchors of the block and of the blocks around it, the nearest first.
    pub anchors: Vec<String>,
    /// The level, for a heading.
    pub heading: Option<u8>,
    /// The table of a cell, as an index into [`View::tables`].
    pub table: Option<usize>,
}

/// A cell in the grid of a table: the unit of its text and the columns it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    pub unit: usize,
    /// The first column, from 0.
    pub first: usize,
    /// One past the last column.
    pub end: usize,
}

/// A table as a grid.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// The anchor of the table block.
    pub anchor: Option<String>,
    pub header_rows: usize,
    /// For each row the cells that cover it, left to right: its own and those that span down
    /// from a row above.
    pub rows: Vec<Vec<Placed>>,
    /// The kind of the region the table was read from, when the document lists it.
    pub region: Option<RegionKind>,
}

/// The last part of a field's full name, without the index some producers add: `Name` for
/// `form[0].page[0].Name[0]`.
pub fn short_name(name: &str) -> String {
    let last = name.rsplit('.').next().unwrap_or(name);
    let last = match last.rsplit_once('[') {
        Some((head, index))
            if index
                .strip_suffix(']')
                .is_some_and(|i| !i.is_empty() && i.bytes().all(|b| b.is_ascii_digit())) =>
        {
            head
        }
        _ => last,
    };
    norm(last)
}

/// A document as the facts see it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct View {
    pub units: Vec<Unit>,
    pub tables: Vec<Table>,
    /// The texts of all units in reading order, joined by spaces.
    pub stream: String,
    /// The targets of the links, each with the unit it stands in.
    pub links: Vec<(String, usize)>,
    /// The mark the amounts of the document part their decimals with.
    pub decimal: Decimal,
    /// The region listed for each anchor: the first, and a figure or a code only when no
    /// other region is listed there.
    regions: HashMap<String, RegionNote>,
}

/// The text of a field's value, when it is one text.
pub fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::Text { value } | Value::Date { value } | Value::DateTime { value } => {
            Some(norm(value))
        }
        Value::Number { value, .. } | Value::Percent { value } => Some(value.to_string()),
        Value::Bool { value } => Some(value.to_string()),
        Value::List { .. } => None,
    }
}

impl View {
    /// The view of `doc`.
    pub fn of(doc: &Document) -> View {
        let mut view = View::default();
        // A figure or a code with no block of its own is listed at the block that holds its
        // text: the region that block was read from goes first.
        let hosted = |r: &RegionNote| matches!(r.kind, RegionKind::Figure | RegionKind::Code);
        for region in &doc.regions {
            match view.regions.get_mut(&region.anchor.0) {
                Some(first) if hosted(first) && !hosted(region) => *first = region.clone(),
                Some(_) => {}
                None => {
                    view.regions.insert(region.anchor.0.clone(), region.clone());
                }
            }
        }
        view.blocks(&doc.body, &[]);
        let texts: Vec<&str> = view
            .units
            .iter()
            .map(|u| u.text.as_str())
            .filter(|t| !t.is_empty())
            .collect();
        view.stream = texts.join(" ");
        view.decimal = decimal_of(&view.stream);
        view
    }

    /// The region the unit stands in: the one listed for its block, else for the nearest
    /// block around it.
    pub fn region_of(&self, unit: usize) -> Option<&RegionNote> {
        self.units
            .get(unit)?
            .anchors
            .iter()
            .find_map(|a| self.regions.get(a))
    }

    /// The region listed at `anchor`.
    pub fn region_at(&self, anchor: &str) -> Option<&RegionNote> {
        self.regions.get(anchor)
    }

    fn blocks(&mut self, blocks: &[Block], around: &[String]) {
        for block in blocks {
            self.block(block, around);
        }
    }

    /// `around` with the anchor of a block before it.
    fn chain(anchor: Option<&rezzer_model::Anchor>, around: &[String]) -> Vec<String> {
        let mut chain: Vec<String> = anchor.iter().map(|a| a.0.clone()).collect();
        chain.extend_from_slice(around);
        chain
    }

    fn block(&mut self, block: &Block, around: &[String]) {
        match block {
            Block::Section { anchor, blocks, .. }
            | Block::Quote { anchor, blocks }
            | Block::Callout { anchor, blocks, .. } => {
                let chain = View::chain(anchor.as_ref(), around);
                self.blocks(blocks, &chain);
            }
            Block::Heading {
                level,
                inlines,
                anchor,
            } => {
                let chain = View::chain(anchor.as_ref(), around);
                self.inlines(inlines, chain, Some(*level), None);
            }
            Block::Paragraph {
                inlines,
                role,
                anchor,
            } => {
                if role.as_deref().is_some_and(|r| OWN_ROLES.contains(&r)) {
                    return;
                }
                let chain = View::chain(anchor.as_ref(), around);
                self.inlines(inlines, chain, None, None);
            }
            Block::Task {
                inlines, anchor, ..
            } => {
                let chain = View::chain(anchor.as_ref(), around);
                self.inlines(inlines, chain, None, None);
            }
            Block::List { items, anchor, .. } => {
                let chain = View::chain(anchor.as_ref(), around);
                for item in items {
                    self.blocks(item, &chain);
                }
            }
            Block::Table {
                caption,
                header_rows,
                rows,
                anchor,
                ..
            } => {
                let chain = View::chain(anchor.as_ref(), around);
                if let Some(caption) = caption {
                    self.unit(lines(caption), chain.clone(), None, None);
                }
                self.table(*header_rows, rows, &chain, anchor.is_some());
            }
            Block::Code { text, anchor, .. } => {
                let chain = View::chain(anchor.as_ref(), around);
                self.unit(lines(text), chain, None, None);
            }
            Block::Math { latex, anchor, .. } => {
                let chain = View::chain(anchor.as_ref(), around);
                self.unit(lines(latex), chain, None, None);
            }
            Block::Spatial {
                lines: laid,
                links,
                anchor,
            } => {
                let chain = View::chain(anchor.as_ref(), around);
                let texts = laid
                    .iter()
                    .map(|l| {
                        let pieces: Vec<&str> = l.pieces.iter().map(|p| p.text.as_str()).collect();
                        norm(&pieces.join(" "))
                    })
                    .collect();
                let unit = self.unit(texts, chain, None, None);
                for target in links {
                    self.links.push((target.trim().to_string(), unit));
                }
            }
            Block::Media { .. } | Block::Rule { .. } => {}
        }
    }

    /// Adds a unit of `lines` and gives its index.
    fn unit(
        &mut self,
        lines: Vec<String>,
        anchors: Vec<String>,
        heading: Option<u8>,
        table: Option<usize>,
    ) -> usize {
        let held: Vec<&str> = lines
            .iter()
            .map(String::as_str)
            .filter(|l| !l.is_empty())
            .collect();
        let text = held.join(" ");
        self.units.push(Unit {
            lines,
            text,
            anchors,
            heading,
            table,
        });
        self.units.len() - 1
    }

    /// Adds the unit of `inlines` with its links.
    fn inlines(
        &mut self,
        inlines: &[Inline],
        anchors: Vec<String>,
        heading: Option<u8>,
        table: Option<usize>,
    ) -> usize {
        let mut raw = String::new();
        let mut targets = Vec::new();
        collect(inlines, &mut raw, &mut targets);
        let unit = self.unit(lines(&raw), anchors, heading, table);
        for target in targets {
            self.links.push((target, unit));
        }
        unit
    }

    /// Adds a table; `own` says that the first anchor of `chain` is the table's own.
    fn table(&mut self, header_rows: usize, rows: &[Vec<Cell>], chain: &[String], own: bool) {
        let index = self.tables.len();
        let region = chain
            .iter()
            .find_map(|a| self.regions.get(a))
            .map(|r| r.kind);
        // Cells that span down into later rows: the placed cell and the rows it still covers.
        let mut down: Vec<(Placed, usize)> = Vec::new();
        let mut grid = Vec::with_capacity(rows.len());
        for row in rows {
            let mut placed: Vec<Placed> = down.iter().map(|(p, _)| *p).collect();
            for (_, left) in &mut down {
                *left -= 1;
            }
            let covered = placed.clone();
            down.retain(|(_, left)| *left > 0);
            let mut column = 0;
            for cell in row {
                while let Some(over) = covered.iter().find(|p| p.first <= column && column < p.end)
                {
                    column = over.end;
                }
                let unit = self.inlines(&cell.inlines, chain.to_vec(), None, Some(index));
                let wide = (cell.col_span as usize).clamp(1, MAX_SPAN);
                let cell_at = Placed {
                    unit,
                    first: column,
                    end: column + wide,
                };
                column += wide;
                placed.push(cell_at);
                let tall = (cell.row_span as usize).clamp(1, MAX_SPAN);
                if tall > 1 {
                    down.push((cell_at, tall - 1));
                }
            }
            placed.sort_by_key(|p| p.first);
            grid.push(placed);
        }
        self.tables.push(Table {
            anchor: chain.first().filter(|_| own).cloned(),
            header_rows,
            rows: grid,
            region,
        });
    }
}

/// The text of `inlines` with a line feed at each line break, and the targets of its links.
/// Status marks, pictures and footnote marks give no text.
fn collect(inlines: &[Inline], text: &mut String, links: &mut Vec<String>) {
    for inline in inlines {
        match inline {
            Inline::Text { text: t, .. } | Inline::Code { text: t } => text.push_str(t),
            Inline::Math { latex } => text.push_str(latex),
            Inline::LineBreak => text.push('\n'),
            Inline::Link {
                target, inlines, ..
            } => {
                links.push(target.trim().to_string());
                collect(inlines, text, links);
            }
            Inline::Image { .. }
            | Inline::Status { .. }
            | Inline::FootnoteRef { .. }
            | Inline::AnchorMark { .. } => {}
        }
    }
}
