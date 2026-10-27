// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Leaf and container blocks: headings, frontmatter, callouts and the closing of frames.

use pulldown_cmark::BlockQuoteKind;
use rezzer_model::text::heading_slug;
use rezzer_model::{
    Anchor, Block, CalloutKind, Cell, Inline, Loss, LossReason, Severity, plain_text,
};

use super::inlines::push_merged;
use super::{Builder, Frame, Leaf};
use crate::frontmatter::{self, FmError, Syntax};

/// Frontmatter over this size is not parsed.
const MAX_FRONTMATTER_BYTES: usize = 1024 * 1024;

/// Frontmatter over this size is kept as text but not parsed. Real frontmatter is tiny. The
/// parser and the writing of the properties are bounded by depth, work and time budgets; this
/// cap is defense in depth.
const MAX_PARSE_BYTES: usize = 256 * 1024;

impl<'b> Builder<'b> {
    /// Ends text that stands directly in a list item, before a block or the item ends.
    pub(super) fn flush_implicit(&mut self) {
        if matches!(self.leaf, Leaf::Implicit) {
            self.finish_inline_leaf();
        }
    }

    /// Ends a paragraph, implicit paragraph or the text of a task item.
    pub(super) fn finish_inline_leaf(&mut self) {
        self.close_inline_frames();
        self.leaf = Leaf::None;
        let inlines = std::mem::take(&mut self.cur);
        let display = std::mem::take(&mut self.display_math);
        let task = match self.stack.last_mut() {
            Some(Frame::Item { task, .. }) => task.take(),
            _ => None,
        };
        if let Some(state) = task {
            self.push_block(Block::Task {
                state,
                inlines,
                anchor: None,
            });
        } else if display && matches!(inlines.as_slice(), [Inline::Math { .. }]) {
            if let Some(Inline::Math { latex }) = inlines.into_iter().next() {
                self.push_block(Block::Math {
                    latex,
                    source_kind: "latex".into(),
                    anchor: None,
                });
            }
        } else if !inlines.is_empty() {
            let brk = self.para_break.take();
            if let Some(Frame::Quote {
                kind: None,
                blocks,
                callout_break,
                ..
            }) = self.stack.last_mut()
                && blocks.is_empty()
            {
                *callout_break = brk;
            }
            self.push_block(Block::Paragraph {
                inlines,
                role: None,
                anchor: None,
            });
        }
    }

    pub(super) fn finish_heading(&mut self, level: u8) {
        self.close_inline_frames();
        self.leaf = Leaf::None;
        self.display_math = false;
        let inlines = std::mem::take(&mut self.cur);
        let text = plain_text(&inlines);
        if level == 1 && self.first_h1.is_none() && !text.trim().is_empty() {
            self.first_h1 = Some(text.trim().to_string());
        }
        let anchor = Anchor(format!("h:{}", self.unique_slug(&text)));
        self.push_block(Block::Heading {
            level,
            inlines,
            anchor: Some(anchor),
        });
    }

    fn unique_slug(&mut self, text: &str) -> String {
        let mut slug = heading_slug(text);
        if slug.is_empty() {
            slug = "section".into();
        }
        let seen = self.heading_slugs.entry(slug.clone()).or_insert(0);
        let unique = if *seen == 0 {
            slug
        } else {
            format!("{slug}-{seen}")
        };
        *seen += 1;
        unique
    }

    /// Reads the frontmatter into the properties. What cannot be read stays visible as a code
    /// block at the top of the body, together with a loss.
    pub(super) fn frontmatter(&mut self, syntax: Syntax, raw: &str) {
        let lang = match syntax {
            Syntax::Yaml => "yaml",
            Syntax::Toml => "toml",
        };
        let problems = if raw.len() > MAX_PARSE_BYTES {
            let cut = if raw.len() > MAX_FRONTMATTER_BYTES {
                ", text cut at 1 MiB"
            } else {
                ""
            };
            vec![(
                LossReason::Limit,
                format!("frontmatter larger than 256 KiB not parsed{cut}"),
            )]
        } else {
            frontmatter::apply(syntax, raw, &mut self.properties, self.budget)
                .into_iter()
                .map(|e| match e {
                    FmError::Damaged(m) => (
                        LossReason::Damaged,
                        format!("frontmatter could not be read, kept as text: {m}"),
                    ),
                    FmError::Limit(m) => (
                        LossReason::Limit,
                        format!("frontmatter not fully read, kept as text: {m}"),
                    ),
                    FmError::Partly(m) => (
                        LossReason::Damaged,
                        format!("frontmatter partly read, kept as text: {m}"),
                    ),
                    FmError::Uncertain(m) => (LossReason::Uncertain, m),
                })
                .collect()
        };
        let keep_text = problems
            .iter()
            .any(|(why, _)| *why != LossReason::Uncertain);
        for (why, what) in problems {
            self.losses.push(Loss {
                location: "frontmatter".into(),
                what,
                why,
                severity: Severity::Warning,
            });
        }
        if keep_text {
            // The raw text stays visible, cut at the size cap on a character boundary.
            let mut end = raw.len().min(MAX_FRONTMATTER_BYTES);
            while !raw.is_char_boundary(end) {
                end -= 1;
            }
            self.push_block(Block::Code {
                lang: Some(lang.into()),
                text: strip_one_newline(raw[..end].to_string()),
                anchor: None,
            });
        }
    }

    pub(super) fn push_block(&mut self, block: Block) {
        if !self.charge() {
            return;
        }
        match self.stack.last_mut() {
            Some(
                Frame::Root { blocks }
                | Frame::Quote { blocks, .. }
                | Frame::Item { blocks, .. }
                | Frame::Footnote { blocks, .. },
            ) => blocks.push(block),
            // A block directly in a list cannot happen; keep it as its own item.
            Some(Frame::List { items, .. }) => items.push(vec![block]),
            None => self.body.push(block),
        }
    }

    pub(super) fn close_top_frame(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        let Some(frame) = self.stack.pop() else {
            return;
        };
        match frame {
            Frame::Root { blocks } => self.body.extend(blocks),
            Frame::Quote {
                kind,
                mut blocks,
                callout_break,
                ..
            } => {
                let block = match kind {
                    Some(k) => Block::Callout {
                        kind: callout_kind(k),
                        title: None,
                        folded: None,
                        blocks,
                        anchor: None,
                    },
                    None => match read_callout_header(&mut blocks, callout_break) {
                        Some((kind, folded, title)) => Block::Callout {
                            kind,
                            title,
                            folded,
                            blocks,
                            anchor: None,
                        },
                        None => {
                            // A footnote definition inside a quote leaves nothing behind.
                            if blocks.is_empty() {
                                return;
                            }
                            if callout_break.is_some()
                                && let Some(Block::Paragraph { inlines, .. }) = blocks.first_mut()
                            {
                                let mut joined = Vec::with_capacity(inlines.len());
                                for inline in std::mem::take(inlines) {
                                    push_merged(&mut joined, inline);
                                }
                                *inlines = joined;
                            }
                            Block::Quote {
                                blocks,
                                anchor: None,
                            }
                        }
                    },
                };
                self.push_block(block);
            }
            Frame::List { ordered, items, .. } => self.push_block(Block::List {
                ordered,
                items,
                anchor: None,
            }),
            Frame::Item {
                mut blocks, task, ..
            } => {
                if let Some(state) = task {
                    // A task item without text.
                    blocks.insert(
                        0,
                        Block::Task {
                            state,
                            inlines: Vec::new(),
                            anchor: None,
                        },
                    );
                }
                match self.stack.last_mut() {
                    Some(Frame::List { items, .. }) => items.push(blocks),
                    _ => blocks.into_iter().for_each(|b| self.push_block(b)),
                }
            }
            Frame::Footnote { label, blocks, .. } => self.footnotes.push((label, blocks)),
        }
    }

    /// Closes everything still open, after the events ended or the time ran out.
    pub(super) fn unwind(&mut self) {
        if matches!(self.leaf, Leaf::Cell) {
            self.close_inline_frames();
            let inlines = std::mem::take(&mut self.cur);
            if let Some(t) = &mut self.table {
                t.row.push(Cell::new(inlines));
            }
            self.leaf = Leaf::None;
        }
        match std::mem::replace(&mut self.leaf, Leaf::None) {
            Leaf::Implicit | Leaf::Paragraph => {
                self.leaf = Leaf::Paragraph;
                self.finish_inline_leaf();
            }
            Leaf::Heading(level) => self.finish_heading(level),
            Leaf::Code { lang, text } => self.push_block(Block::Code {
                lang,
                text: strip_one_newline(text),
                anchor: None,
            }),
            Leaf::Html(text) => self.push_block(Block::Code {
                lang: Some("html".into()),
                text: strip_one_newline(text),
                anchor: None,
            }),
            Leaf::Cell | Leaf::None => {}
        }
        if let Some(mut t) = self.table.take() {
            if !t.row.is_empty() {
                let row = std::mem::take(&mut t.row);
                t.rows.push(row);
            }
            self.push_block(Block::Table {
                caption: None,
                header_rows: t.header_rows,
                role: None,
                rows: t.rows,
                alignments: t.alignments,
                anchor: None,
                layout: None,
            });
        }
        while self.stack.len() > 1 {
            self.close_top_frame();
        }
        if let Some(Frame::Root { blocks }) = self.stack.pop() {
            self.body.extend(blocks);
        }
        // Name what really stopped the read: the element budget first, then the clock. The
        // input cut only counts when nothing earlier stopped the read, because then the
        // parser never got to the cut.
        if self.elements_stopped {
            self.losses.push(Loss {
                location: "document".into(),
                what: "element limit reached, rest of the document not read".into(),
                why: LossReason::Limit,
                severity: Severity::Warning,
            });
        } else if self.time_stopped {
            self.losses.push(Loss {
                location: "document".into(),
                what: "time limit reached, rest of the document not read".into(),
                why: LossReason::Limit,
                severity: Severity::Warning,
            });
        } else if let Some(line) = self.input_cut {
            self.losses.push(Loss {
                location: format!("line {line}"),
                what: "element limit reached, rest of the document not read".into(),
                why: LossReason::Limit,
                severity: Severity::Warning,
            });
        }
        if self.empty_math > 1
            && let Some(loss) = self.empty_math_loss.and_then(|i| self.losses.get_mut(i))
        {
            loss.what = format!("{} (in {} places)", loss.what, self.empty_math);
        }
        if self.skipped_subtrees > 1
            && let Some(loss) = self.depth_loss.and_then(|i| self.losses.get_mut(i))
        {
            loss.what = format!("{} (in {} places)", loss.what, self.skipped_subtrees);
        }
    }
}

/// Reads `[!KIND]`, an optional fold sign and a title from the first paragraph of a quote, the
/// callout form Rezzer writes itself (and Obsidian does). On a match the header is removed from
/// `blocks[0]`; the rest of that paragraph stays as body.
fn read_callout_header(
    blocks: &mut Vec<Block>,
    brk: Option<usize>,
) -> Option<(CalloutKind, Option<bool>, Option<String>)> {
    let Some(Block::Paragraph { inlines, .. }) = blocks.first_mut() else {
        return None;
    };
    let Some(Inline::Text { text, .. }) = inlines.first() else {
        return None;
    };
    let rest = text.strip_prefix("[!")?;
    let close = rest.find(']')?;
    let kind = match rest[..close].to_ascii_lowercase().as_str() {
        "note" => CalloutKind::Note,
        "tip" => CalloutKind::Tip,
        "important" => CalloutKind::Important,
        "warning" => CalloutKind::Warning,
        "caution" => CalloutKind::Caution,
        "details" => CalloutKind::Details,
        _ => return None,
    };
    let mut after = &rest[close + 1..];
    let folded = match after.chars().next() {
        Some('-') => Some(true),
        Some('+') => Some(false),
        _ => None,
    };
    if folded.is_some() {
        after = &after[1..];
    }
    if !after.is_empty() && !after.starts_with(char::is_whitespace) {
        return None;
    }
    let split = brk.unwrap_or(inlines.len()).min(inlines.len());
    let mut header: Vec<Inline> = inlines[..split].to_vec();
    if let Some(Inline::Text { text, .. }) = header.first_mut() {
        *text = after.to_string();
    }
    let title = plain_text(&header).trim().to_string();
    // The break is a space inline that later text may have been joined to.
    let mut body: Vec<Inline> = inlines.get(split..).unwrap_or_default().to_vec();
    if brk.is_some() {
        match body.first_mut() {
            Some(Inline::LineBreak) => {
                body.remove(0);
            }
            Some(Inline::Text { text, .. }) => {
                *text = text.strip_prefix(' ').unwrap_or(text).to_string();
                if text.is_empty() {
                    body.remove(0);
                }
            }
            _ => {}
        }
    }
    if body.is_empty() {
        blocks.remove(0);
    } else if let Some(Block::Paragraph { inlines, .. }) = blocks.first_mut() {
        *inlines = body;
    }
    Some((kind, folded, Some(title).filter(|t| !t.is_empty())))
}

fn callout_kind(kind: BlockQuoteKind) -> CalloutKind {
    match kind {
        BlockQuoteKind::Note => CalloutKind::Note,
        BlockQuoteKind::Tip => CalloutKind::Tip,
        BlockQuoteKind::Important => CalloutKind::Important,
        BlockQuoteKind::Warning => CalloutKind::Warning,
        BlockQuoteKind::Caution => CalloutKind::Caution,
    }
}

pub(super) fn strip_one_newline(mut text: String) -> String {
    if text.ends_with('\n') {
        text.pop();
    }
    text
}
