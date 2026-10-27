// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reading the body of a document: sections, pages, chapters, footnotes and anchors.

use super::*;

/// The body without the leading level-1 heading that is the title (as the content writes
/// it).
pub(super) fn content_body<'d>(doc: &'d Document, title: &str) -> &'d [Block] {
    match doc.body.first() {
        Some(Block::Heading {
            level: 1, inlines, ..
        }) if rezzer_model::plain_text(inlines).trim() == title => &doc.body[1..],
        _ => &doc.body,
    }
}

/// The page number of a page section, from its anchor `p<n>`.
pub(super) fn page_number(b: &Block) -> Option<u32> {
    match b {
        Block::Section {
            kind: SectionKind::Page,
            anchor,
            ..
        } => anchor
            .as_ref()
            .and_then(|a| a.0.strip_prefix('p'))
            .and_then(|n| n.parse().ok()),
        _ => None,
    }
}

/// The outline's top-level chapters with a page, in page order, one per page (the first
/// title wins).
pub(super) fn chapters(doc: &Document) -> Vec<(u32, &str)> {
    let mut list: Vec<(u32, &str)> = doc
        .facets
        .outline
        .iter()
        .filter(|e| e.level == 1)
        .filter_map(|e| e.page.map(|p| (p, e.title.as_str())))
        .collect();
    list.sort_by_key(|(p, _)| *p);
    list.dedup_by_key(|(p, _)| *p);
    list
}

/// True for the Outline section of a PDF: a trailing unit of its own.
pub(super) fn is_pdf_outline(doc: &Document, b: &Block) -> bool {
    doc.source.format == FormatId::Pdf
        && matches!(b, Block::Section {
            kind: SectionKind::Other,
            title: Some(title),
            ..
        } if title == "Outline")
}

/// A run of the body's top level for the units: blocks of the body, or a section made anew
/// (the footnote definitions no unit cites).
pub(super) enum Run {
    Blocks(usize, usize),
    Owned(Block),
}

/// The definitions of the Markdown reader's `Footnotes` section: a titled section that holds
/// only footnote definitions.
pub(super) fn footnotes_section(b: &Block) -> Option<&[Block]> {
    match b {
        Block::Section {
            title: Some(title),
            blocks,
            ..
        } if title == "Footnotes"
            && !blocks.is_empty()
            && blocks.iter().all(|d| footnote_label(d).is_some()) =>
        {
            Some(blocks)
        }
        _ => None,
    }
}

/// The label of a footnote definition (`fn:<label>`, no title).
pub(super) fn footnote_label(b: &Block) -> Option<&str> {
    match b {
        Block::Section {
            title: None,
            anchor: Some(anchor),
            ..
        } => anchor.0.strip_prefix("fn:"),
        _ => None,
    }
}

/// A section that is a unit of its own: any section but a footnote definition.
pub(super) fn is_section(b: &Block) -> bool {
    matches!(b, Block::Section { .. }) && footnote_label(b).is_none()
}

pub(super) fn heading_level(b: &Block) -> Option<u8> {
    match b {
        Block::Heading { level, .. } => Some(*level),
        _ => None,
    }
}

/// A block that stays with the block after it: a heading or a caption.
pub(super) fn leads(b: &Block) -> bool {
    match b {
        Block::Heading { .. } => true,
        Block::Paragraph {
            role: Some(role), ..
        } => role == "caption",
        _ => false,
    }
}

pub(super) fn is_note(b: &Block) -> bool {
    matches!(b, Block::Paragraph { role: Some(role), .. } if role == "note")
}

/// The part of a row range name that tells the table, when its list holds more than one
/// table: `-<key slug>` from the layout key without its sheet (`table:Sales` gives
/// `-sales`, `Data!B5` gives `-b5`), else `-table-<n>`.
pub(super) fn table_key(list: &[Block], table: &Block) -> String {
    let tables: Vec<&Block> = list
        .iter()
        .filter(|b| matches!(b, Block::Table { .. }))
        .collect();
    if tables.len() < 2 {
        return String::new();
    }
    if let Block::Table {
        layout: Some(layout),
        ..
    } = table
    {
        let key = layout.key.rsplit(['!', ':']).next().unwrap_or(&layout.key);
        let key = slug(key);
        let key = cut_slug(&key, KEY_CAP);
        if !key.is_empty() {
            return format!("-{key}");
        }
    }
    let n = tables
        .iter()
        .position(|t| std::ptr::eq(*t, table))
        .map_or(1, |i| i + 1);
    format!("-table-{n}")
}

/// `wraps` around the text `body` makes, outermost first; a page's own notes only where the
/// unit reaches the page's end (`last`).
pub(super) fn wrapped(
    wraps: &[Wrap<'_>],
    last: bool,
    ctx: &Ctx,
    depth: usize,
    level: u8,
    body: &dyn Fn(usize, u8) -> String,
) -> String {
    match wraps.split_first() {
        None => body(depth, level),
        Some((w, rest)) => blocks::section(
            w.kind,
            w.title,
            w.anchor.filter(|_| last),
            ctx,
            depth,
            level,
            &|d, l| wrapped(rest, last, ctx, d, l, body),
        ),
    }
}

/// A node of the model a walk visits.
#[derive(Clone, Copy)]
pub(super) enum Node<'x> {
    Block(&'x Block),
    Inline(&'x Inline),
}

/// Visits every block and inline of `list`, nested ones included (cells too), down to
/// [`MAX_DEPTH`].
pub(super) fn walk<'x>(list: &'x [Block], depth: usize, f: &mut dyn FnMut(Node<'x>)) {
    if depth > MAX_DEPTH {
        return;
    }
    for b in list {
        f(Node::Block(b));
        match b {
            Block::Section { blocks, .. }
            | Block::Quote { blocks, .. }
            | Block::Callout { blocks, .. } => walk(blocks, depth + 1, f),
            Block::List { items, .. } => {
                for item in items {
                    walk(item, depth + 1, f);
                }
            }
            Block::Paragraph { inlines, .. }
            | Block::Heading { inlines, .. }
            | Block::Task { inlines, .. } => walk_inlines(inlines, depth + 1, f),
            Block::Table { rows, .. } => {
                for cell in rows.iter().flatten() {
                    walk_inlines(&cell.inlines, depth + 1, f);
                }
            }
            Block::Code { .. } | Block::Math { .. } | Block::Media { .. } | Block::Rule { .. } => {}
        }
    }
}

pub(super) fn walk_inlines<'x>(list: &'x [Inline], depth: usize, f: &mut dyn FnMut(Node<'x>)) {
    if depth > MAX_DEPTH {
        return;
    }
    for i in list {
        f(Node::Inline(i));
        if let Inline::Link { inlines, .. } = i {
            walk_inlines(inlines, depth + 1, f);
        }
    }
}

/// The footnote labels `list` cites.
pub(super) fn cited_footnotes(list: &[Block], depth: usize, out: &mut HashSet<String>) {
    walk(list, depth, &mut |node| {
        if let Node::Inline(Inline::FootnoteRef { id }) = node
            && !out.contains(id.as_str())
        {
            out.insert(id.clone());
        }
    });
}

/// Every anchor `list` holds: of its blocks and sections and its inline anchor marks.
pub(super) fn collect_anchors<'x>(list: &'x [Block], depth: usize, f: &mut dyn FnMut(&'x str)) {
    walk(list, depth, &mut |node| match node {
        Node::Block(b) => {
            if let Some(anchor) = block_anchor(b) {
                f(anchor);
            }
        }
        Node::Inline(Inline::AnchorMark { anchor }) => f(&anchor.0),
        Node::Inline(_) => {}
    });
}

fn block_anchor(b: &Block) -> Option<&str> {
    match b {
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
        | Block::Rule { anchor, .. }
        | Block::Task { anchor, .. } => anchor.as_ref().map(|a| a.0.as_str()),
    }
}
