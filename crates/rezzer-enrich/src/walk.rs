// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Iterative walks over blocks and inlines. A hostile file can nest blocks very deeply, so
//! nothing here recurses.

use rezzer_model::{Block, Inline};

/// Visits every block of `body` in document order, children after their parent. The visitor
/// returns `false` to stop the walk.
pub(crate) fn blocks<'a>(body: &'a [Block], mut visit: impl FnMut(&'a Block) -> bool) {
    let mut stack: Vec<&'a Block> = body.iter().rev().collect();
    while let Some(block) = stack.pop() {
        crate::work::add(1);
        if !visit(block) {
            return;
        }
        match block {
            Block::Section { blocks, .. }
            | Block::Quote { blocks, .. }
            | Block::Callout { blocks, .. } => stack.extend(blocks.iter().rev()),
            Block::List { items, .. } => {
                stack.extend(items.iter().rev().flat_map(|item| item.iter().rev()));
            }
            _ => {}
        }
    }
}

/// Calls `f` for each inline sequence a block holds directly: paragraph, heading and task
/// content and every table cell.
pub(crate) fn block_inlines<'a>(block: &'a Block, mut f: impl FnMut(&'a [Inline])) {
    match block {
        Block::Heading { inlines, .. }
        | Block::Paragraph { inlines, .. }
        | Block::Task { inlines, .. } => f(inlines),
        Block::Table { rows, .. } => {
            for cell in rows.iter().flatten() {
                f(&cell.inlines);
            }
        }
        _ => {}
    }
}

/// Visits every inline of `list`, links' content included, in document order.
pub(crate) fn inlines<'a>(list: &'a [Inline], mut visit: impl FnMut(&'a Inline)) {
    let mut stack: Vec<&'a Inline> = list.iter().rev().collect();
    while let Some(inline) = stack.pop() {
        crate::work::add(1);
        visit(inline);
        if let Inline::Link { inlines, .. } = inline {
            stack.extend(inlines.iter().rev());
        }
    }
}
