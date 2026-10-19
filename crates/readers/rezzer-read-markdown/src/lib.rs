// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Markdown reader: CommonMark with GFM tables, alerts and task lists, footnotes, math,
//! wikilinks and YAML or TOML frontmatter, read into the document model.
//!
//! The parser events are consumed by an explicit stack, never by recursion, so nesting of any
//! depth cannot overflow the call stack. Containers (block quote, list, list item, footnote
//! definition) are charged to the depth budget; a subtree over the limit is skipped and
//! reported as a loss.

mod frontmatter;

use std::collections::{HashMap, HashSet};

use pulldown_cmark::{
    BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, LinkType, MetadataBlockKind, Options,
    Parser, Tag, TagEnd,
};
use rezzer_limits::{Budget, DepthGuard};
use rezzer_model::{
    Anchor, Block, CalloutKind, Cell, Completeness, Document, Edge, FormatId, Graph, Inline, Loss,
    LossReason, Node, ReadOptions, ReadOutcome, Reader, ReaderInfo, SectionKind, Severity, Source,
    TaskState, source_info,
};

use frontmatter::Syntax;

/// How many parser events are handled between two time checks.
const CHECK_EVERY: usize = 1024;

/// Frontmatter over this size is not parsed.
const MAX_FRONTMATTER_BYTES: usize = 1024 * 1024;

/// Reads Markdown. The encoding is detected; existing frontmatter becomes properties, GFM
/// alerts become callouts and links and wikilinks form a link graph.
#[derive(Debug, Clone, Copy, Default)]
pub struct MarkdownReader;

impl Reader for MarkdownReader {
    fn id(&self) -> &'static str {
        "markdown"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn formats(&self) -> &'static [FormatId] {
        &[FormatId::Markdown]
    }

    fn read(&self, src: &Source, _opts: &ReadOptions, budget: &Budget) -> ReadOutcome {
        let decoded = textenc::decode(src.bytes());
        let mut info = source_info(src);
        info.encoding = Some(decoded.encoding.to_string());
        info.reader = Some(ReaderInfo {
            id: self.id().to_string(),
            version: self.version().to_string(),
        });
        let mut doc = Document::new(info);

        if !decoded.confident {
            doc.losses.push(Loss {
                location: "document".into(),
                what: "some bytes could not be decoded and were replaced with U+FFFD".into(),
                why: LossReason::Damaged,
                severity: Severity::Warning,
            });
        }

        let mut builder = Builder::new(budget);
        builder.run(&decoded.text);
        let first_h1 = builder.first_h1.take();
        builder.finish_into(&mut doc);
        if doc.properties.title.is_none() {
            doc.properties.title = first_h1;
        }
        builder.add_graph(&mut doc, &src.name);

        let completeness = if doc
            .losses
            .iter()
            .any(|l| matches!(l.why, LossReason::Limit | LossReason::Damaged))
        {
            Completeness::Partial
        } else {
            Completeness::Complete
        };
        ReadOutcome {
            document: doc,
            completeness,
        }
    }
}

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS
        | Options::ENABLE_MATH
        | Options::ENABLE_WIKILINKS
        | Options::ENABLE_GFM
}

/// An open container block.
enum Frame<'b> {
    Root {
        blocks: Vec<Block>,
    },
    Quote {
        kind: Option<BlockQuoteKind>,
        blocks: Vec<Block>,
        _guard: DepthGuard<'b>,
    },
    List {
        ordered: bool,
        items: Vec<Vec<Block>>,
        _guard: DepthGuard<'b>,
    },
    Item {
        blocks: Vec<Block>,
        task: Option<TaskState>,
        _guard: DepthGuard<'b>,
    },
    Footnote {
        label: String,
        blocks: Vec<Block>,
        _guard: DepthGuard<'b>,
    },
}

/// The leaf block whose content is being collected.
enum Leaf {
    None,
    /// Text directly in a tight list item, without a paragraph event.
    Implicit,
    Paragraph,
    Heading(u8),
    Cell,
    Code {
        lang: Option<String>,
        text: String,
    },
    Html(String),
    Meta {
        syntax: Syntax,
        text: String,
    },
}

/// An open link or image inside a leaf block.
enum InlineFrame {
    Link {
        target: String,
        title: Option<String>,
        internal: bool,
        inlines: Vec<Inline>,
    },
    Image {
        marker: String,
        target: String,
        title: String,
        alt: String,
    },
}

#[derive(Default)]
struct TableState {
    rows: Vec<Vec<Cell>>,
    header_rows: usize,
    row: Vec<Cell>,
}

struct Builder<'b> {
    budget: &'b Budget,
    stack: Vec<Frame<'b>>,
    leaf: Leaf,
    cur: Vec<Inline>,
    inline_stack: Vec<InlineFrame>,
    emphasis: u32,
    strong: u32,
    /// True when the current inlines hold a display formula from `$$ ... $$`.
    display_math: bool,
    table: Option<TableState>,
    /// Nesting counter while a subtree over the depth limit is skipped.
    skip: usize,
    skipped_subtrees: usize,
    time_stopped: bool,
    footnotes: Vec<(String, Vec<Block>)>,
    heading_slugs: HashMap<String, u32>,
    first_h1: Option<String>,
    frontmatter_seen: bool,
    properties: rezzer_model::Properties,
    losses: Vec<Loss>,
    nodes: Vec<Node>,
    node_ids: HashSet<String>,
    edges: Vec<Edge>,
    body: Vec<Block>,
}

impl<'b> Builder<'b> {
    fn new(budget: &'b Budget) -> Self {
        Builder {
            budget,
            stack: vec![Frame::Root { blocks: Vec::new() }],
            leaf: Leaf::None,
            cur: Vec::new(),
            inline_stack: Vec::new(),
            emphasis: 0,
            strong: 0,
            display_math: false,
            table: None,
            skip: 0,
            skipped_subtrees: 0,
            time_stopped: false,
            footnotes: Vec::new(),
            heading_slugs: HashMap::new(),
            first_h1: None,
            frontmatter_seen: false,
            properties: rezzer_model::Properties::default(),
            losses: Vec::new(),
            nodes: Vec::new(),
            node_ids: HashSet::new(),
            edges: Vec::new(),
            body: Vec::new(),
        }
    }

    fn run(&mut self, text: &str) {
        for (n, event) in Parser::new_ext(text, options()).enumerate() {
            if n % CHECK_EVERY == CHECK_EVERY - 1 && self.budget.check_time().is_err() {
                self.time_stopped = true;
                break;
            }
            self.event(event);
        }
        self.unwind();
    }

    fn event(&mut self, event: Event<'_>) {
        if self.skip > 0 {
            match event {
                Event::Start(_) => self.skip += 1,
                Event::End(_) => self.skip -= 1,
                _ => {}
            }
            return;
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(end) => self.end(end),
            Event::Text(s) => self.on_text(&s),
            Event::Code(s) => {
                self.ensure_inline_leaf();
                self.push_inline(Inline::Code {
                    text: s.into_string(),
                });
            }
            Event::InlineMath(s) => {
                self.ensure_inline_leaf();
                self.push_inline(Inline::Math {
                    latex: s.into_string(),
                });
            }
            Event::DisplayMath(s) => {
                self.ensure_inline_leaf();
                self.display_math = true;
                self.push_inline(Inline::Math {
                    latex: s.into_string(),
                });
            }
            Event::Html(s) => match &mut self.leaf {
                Leaf::Html(text) => text.push_str(&s),
                _ => self.on_text(&s),
            },
            Event::InlineHtml(s) => self.on_text(&s),
            Event::FootnoteReference(label) => {
                self.ensure_inline_leaf();
                self.push_inline(Inline::FootnoteRef {
                    id: label.into_string(),
                });
            }
            Event::SoftBreak | Event::HardBreak => self.on_text("\n"),
            // A thematic break carries no content and the model has no block for it.
            Event::Rule => self.flush_implicit(),
            Event::TaskListMarker(done) => {
                if let Some(Frame::Item { task, .. }) = self.stack.last_mut() {
                    *task = Some(if done {
                        TaskState::Done
                    } else {
                        TaskState::Open
                    });
                }
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                self.flush_implicit();
                self.leaf = Leaf::Paragraph;
            }
            Tag::Heading { level, .. } => {
                self.flush_implicit();
                self.leaf = Leaf::Heading(heading_level(level));
            }
            Tag::BlockQuote(kind) => {
                self.flush_implicit();
                if let Some(_guard) = self.enter() {
                    self.stack.push(Frame::Quote {
                        kind,
                        blocks: Vec::new(),
                        _guard,
                    });
                }
            }
            Tag::CodeBlock(kind) => {
                self.flush_implicit();
                let lang = match kind {
                    CodeBlockKind::Indented => None,
                    CodeBlockKind::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        // `rust,ignore` and `rust {.numbered}`: the language is the first word.
                        .map(|w| w.split([',', '{']).next().unwrap_or(w))
                        .filter(|w| !w.is_empty())
                        .map(String::from),
                };
                self.leaf = Leaf::Code {
                    lang,
                    text: String::new(),
                };
            }
            Tag::HtmlBlock => {
                self.flush_implicit();
                self.leaf = Leaf::Html(String::new());
            }
            Tag::List(start) => {
                self.flush_implicit();
                if let Some(_guard) = self.enter() {
                    self.stack.push(Frame::List {
                        ordered: start.is_some(),
                        items: Vec::new(),
                        _guard,
                    });
                }
            }
            Tag::Item => {
                if let Some(_guard) = self.enter() {
                    self.stack.push(Frame::Item {
                        blocks: Vec::new(),
                        task: None,
                        _guard,
                    });
                }
            }
            Tag::FootnoteDefinition(label) => {
                self.flush_implicit();
                if let Some(_guard) = self.enter() {
                    self.stack.push(Frame::Footnote {
                        label: label.into_string(),
                        blocks: Vec::new(),
                        _guard,
                    });
                }
            }
            Tag::Table(_) => {
                self.flush_implicit();
                self.table = Some(TableState::default());
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(t) = &mut self.table {
                    t.row = Vec::new();
                }
            }
            Tag::TableCell => {
                self.leaf = Leaf::Cell;
                self.cur.clear();
            }
            Tag::Emphasis => {
                self.ensure_inline_leaf();
                self.emphasis += 1;
            }
            Tag::Strong => {
                self.ensure_inline_leaf();
                self.strong += 1;
            }
            Tag::Link {
                link_type,
                dest_url,
                title,
                ..
            } => {
                self.ensure_inline_leaf();
                let wikilink = matches!(link_type, LinkType::WikiLink { .. });
                let mut target = dest_url.into_string();
                if matches!(link_type, LinkType::Email) {
                    target.insert_str(0, "mailto:");
                }
                let internal = wikilink || is_internal(&target);
                self.inline_stack.push(InlineFrame::Link {
                    target,
                    title: Some(title.into_string()).filter(|t| !t.is_empty()),
                    internal,
                    inlines: Vec::new(),
                });
            }
            Tag::Image {
                link_type,
                dest_url,
                title,
                ..
            } => {
                self.ensure_inline_leaf();
                self.inline_stack.push(InlineFrame::Image {
                    marker: if matches!(link_type, LinkType::WikiLink { .. }) {
                        "wiki".into()
                    } else {
                        String::new()
                    },
                    target: dest_url.into_string(),
                    title: title.into_string(),
                    alt: String::new(),
                });
            }
            Tag::MetadataBlock(kind) => {
                self.flush_implicit();
                self.leaf = Leaf::Meta {
                    syntax: match kind {
                        MetadataBlockKind::YamlStyle => Syntax::Yaml,
                        MetadataBlockKind::PlusesStyle => Syntax::Toml,
                    },
                    text: String::new(),
                };
            }
            // Not enabled in `options()`; kept as plain text if they ever appear.
            Tag::Strikethrough
            | Tag::Superscript
            | Tag::Subscript
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition => {}
        }
    }

    fn end(&mut self, end: TagEnd) {
        match end {
            TagEnd::Paragraph => self.finish_inline_leaf(),
            TagEnd::Heading(level) => self.finish_heading(heading_level(level)),
            TagEnd::BlockQuote(_) => {
                self.flush_implicit();
                self.close_top_frame();
            }
            TagEnd::CodeBlock => {
                if let Leaf::Code { lang, text } = std::mem::replace(&mut self.leaf, Leaf::None) {
                    self.push_block(Block::Code {
                        lang,
                        text: strip_one_newline(text),
                        anchor: None,
                    });
                }
            }
            TagEnd::HtmlBlock => {
                if let Leaf::Html(text) = std::mem::replace(&mut self.leaf, Leaf::None) {
                    self.push_block(Block::Code {
                        lang: Some("html".into()),
                        text: strip_one_newline(text),
                        anchor: None,
                    });
                }
            }
            TagEnd::List(_) | TagEnd::Item | TagEnd::FootnoteDefinition => {
                self.flush_implicit();
                self.close_top_frame();
            }
            TagEnd::Table => {
                if let Some(t) = self.table.take() {
                    self.push_block(Block::Table {
                        caption: None,
                        header_rows: t.header_rows,
                        rows: t.rows,
                        alignments: Vec::new(),
                        anchor: None,
                    });
                }
            }
            TagEnd::TableHead => {
                if let Some(t) = &mut self.table {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push(row);
                    t.header_rows = t.rows.len();
                }
            }
            TagEnd::TableRow => {
                if let Some(t) = &mut self.table {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push(row);
                }
            }
            TagEnd::TableCell => {
                self.close_inline_frames();
                let inlines = std::mem::take(&mut self.cur);
                self.leaf = Leaf::None;
                if let Some(t) = &mut self.table {
                    t.row.push(Cell::new(inlines));
                }
            }
            TagEnd::Emphasis => self.emphasis = self.emphasis.saturating_sub(1),
            TagEnd::Strong => self.strong = self.strong.saturating_sub(1),
            TagEnd::Link | TagEnd::Image => self.end_inline_frame(),
            TagEnd::MetadataBlock(_) => self.finish_meta(),
            TagEnd::Strikethrough
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition => {}
        }
    }

    /// Charges one container level. On a limit hit the subtree that starts with the current
    /// event is skipped and counted as one loss.
    fn enter(&mut self) -> Option<DepthGuard<'b>> {
        match self.budget.enter() {
            Ok(guard) => Some(guard),
            Err(hit) => {
                self.skip = 1;
                if self.skipped_subtrees == 0 {
                    self.losses.push(Loss {
                        location: "nested content".into(),
                        what: format!("content nested deeper than {} levels not read", hit.max),
                        why: LossReason::Limit,
                        severity: Severity::Warning,
                    });
                }
                self.skipped_subtrees += 1;
                None
            }
        }
    }

    // --- inline content -------------------------------------------------------------------

    fn on_text(&mut self, s: &str) {
        match &mut self.leaf {
            Leaf::Code { text, .. } | Leaf::Html(text) | Leaf::Meta { text, .. } => {
                text.push_str(s);
            }
            _ => {
                self.ensure_inline_leaf();
                self.push_inline(Inline::Text {
                    text: s.to_string(),
                    emphasis: self.emphasis > 0,
                    strong: self.strong > 0,
                    strike: false,
                });
            }
        }
    }

    fn ensure_inline_leaf(&mut self) {
        if matches!(self.leaf, Leaf::None) {
            self.leaf = Leaf::Implicit;
        }
    }

    fn push_inline(&mut self, inline: Inline) {
        match self.inline_stack.last_mut() {
            Some(InlineFrame::Image { alt, .. }) => alt.push_str(&plain_text(&[inline])),
            Some(InlineFrame::Link { inlines, .. }) => push_merged(inlines, inline),
            None => push_merged(&mut self.cur, inline),
        }
    }

    fn end_inline_frame(&mut self) {
        match self.inline_stack.pop() {
            Some(InlineFrame::Link {
                target,
                title,
                internal,
                inlines,
            }) => {
                self.record_link(&target, internal);
                self.push_inline(Inline::Link {
                    target,
                    title,
                    internal,
                    inlines,
                });
            }
            Some(InlineFrame::Image {
                marker,
                target,
                title,
                alt,
            }) => {
                // The model has no inline picture: the reference stays as source text.
                let text = if marker == "wiki" {
                    format!("![[{target}]]")
                } else if title.is_empty() {
                    format!("![{alt}]({target})")
                } else {
                    format!("![{alt}]({target} \"{title}\")")
                };
                self.push_inline(Inline::Text {
                    text,
                    emphasis: false,
                    strong: false,
                    strike: false,
                });
            }
            None => {}
        }
    }

    fn close_inline_frames(&mut self) {
        while !self.inline_stack.is_empty() {
            self.end_inline_frame();
        }
    }

    // --- leaf blocks ----------------------------------------------------------------------

    /// Ends text that stands directly in a list item, before a block or the item ends.
    fn flush_implicit(&mut self) {
        if matches!(self.leaf, Leaf::Implicit) {
            self.finish_inline_leaf();
        }
    }

    /// Ends a paragraph, implicit paragraph or the text of a task item.
    fn finish_inline_leaf(&mut self) {
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
            self.push_block(Block::Paragraph {
                inlines,
                role: None,
                anchor: None,
            });
        }
    }

    fn finish_heading(&mut self, level: u8) {
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
        let mut slug = slugify(text);
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

    fn finish_meta(&mut self) {
        let Leaf::Meta { syntax, text } = std::mem::replace(&mut self.leaf, Leaf::None) else {
            return;
        };
        if self.frontmatter_seen {
            // Only the first block is the document's frontmatter; keep later ones as code.
            self.push_block(Block::Code {
                lang: Some(
                    match syntax {
                        Syntax::Yaml => "yaml",
                        Syntax::Toml => "toml",
                    }
                    .into(),
                ),
                text: strip_one_newline(text),
                anchor: None,
            });
            return;
        }
        self.frontmatter_seen = true;
        let result = if text.len() > MAX_FRONTMATTER_BYTES {
            self.losses.push(Loss {
                location: "frontmatter".into(),
                what: format!("frontmatter over {MAX_FRONTMATTER_BYTES} bytes not read"),
                why: LossReason::Limit,
                severity: Severity::Warning,
            });
            return;
        } else {
            frontmatter::apply(syntax, &text, &mut self.properties)
        };
        if let Err(e) = result {
            self.losses.push(Loss {
                location: "frontmatter".into(),
                what: format!("frontmatter could not be read: {e}"),
                why: LossReason::Damaged,
                severity: Severity::Warning,
            });
        }
    }

    // --- container blocks -----------------------------------------------------------------

    fn push_block(&mut self, block: Block) {
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

    fn close_top_frame(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        let Some(frame) = self.stack.pop() else {
            return;
        };
        match frame {
            Frame::Root { blocks } => self.body.extend(blocks),
            Frame::Quote { kind, blocks, .. } => self.push_block(match kind {
                Some(k) => Block::Callout {
                    kind: callout_kind(k),
                    title: None,
                    folded: None,
                    blocks,
                    anchor: None,
                },
                None => Block::Quote {
                    blocks,
                    anchor: None,
                },
            }),
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
    fn unwind(&mut self) {
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
            Leaf::Meta { .. } | Leaf::Cell | Leaf::None => {}
        }
        if let Some(mut t) = self.table.take() {
            if !t.row.is_empty() {
                let row = std::mem::take(&mut t.row);
                t.rows.push(row);
            }
            self.push_block(Block::Table {
                caption: None,
                header_rows: t.header_rows,
                rows: t.rows,
                alignments: Vec::new(),
                anchor: None,
            });
        }
        while self.stack.len() > 1 {
            self.close_top_frame();
        }
        if let Some(Frame::Root { blocks }) = self.stack.pop() {
            self.body.extend(blocks);
        }
        if self.time_stopped {
            self.losses.push(Loss {
                location: "document".into(),
                what: "rest of the document not read".into(),
                why: LossReason::Limit,
                severity: Severity::Warning,
            });
        }
        if self.skipped_subtrees > 1
            && let Some(loss) = self
                .losses
                .iter_mut()
                .find(|l| l.location == "nested content")
        {
            loss.what = format!("{} (in {} places)", loss.what, self.skipped_subtrees);
        }
    }

    // --- link graph -----------------------------------------------------------------------

    fn record_link(&mut self, target: &str, internal: bool) {
        if target.is_empty() {
            return;
        }
        // "self" is the document node, so a target that is literally "self" gets another id.
        let id = if target == "self" {
            "link:self".to_string()
        } else {
            target.to_string()
        };
        if !self.node_ids.insert(id.clone()) {
            return;
        }
        let kind = if target.starts_with('#') {
            "anchor"
        } else if internal {
            "path"
        } else {
            "url"
        };
        self.nodes.push(Node {
            id: id.clone(),
            label: target.to_string(),
            kind: Some(kind.into()),
            anchor: None,
        });
        self.edges.push(Edge {
            from: "self".into(),
            to: id,
            kind: "links_to".into(),
            anchor: None,
        });
    }

    /// Moves the collected properties, losses and body into the document.
    fn finish_into(&mut self, doc: &mut Document) {
        doc.properties = std::mem::take(&mut self.properties);
        doc.losses.append(&mut self.losses);
        doc.body = std::mem::take(&mut self.body);
        if !self.footnotes.is_empty() {
            let blocks = std::mem::take(&mut self.footnotes)
                .into_iter()
                .map(|(label, blocks)| Block::Section {
                    kind: SectionKind::Other,
                    title: None,
                    anchor: Some(Anchor(format!("fn:{label}"))),
                    blocks,
                })
                .collect();
            doc.body.push(Block::Section {
                kind: SectionKind::Other,
                title: Some("Footnotes".into()),
                anchor: None,
                blocks,
            });
        }
    }

    fn add_graph(&mut self, doc: &mut Document, name: &str) {
        if self.edges.is_empty() {
            return;
        }
        let mut nodes = vec![Node {
            id: "self".into(),
            label: doc
                .properties
                .title
                .clone()
                .unwrap_or_else(|| name.to_string()),
            kind: Some("document".into()),
            anchor: None,
        }];
        nodes.append(&mut self.nodes);
        doc.facets.graphs.push(Graph {
            id: "links".into(),
            kind: "link".into(),
            nodes,
            edges: std::mem::take(&mut self.edges),
        });
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
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

/// Appends an inline, joining plain text of the same style with the previous one.
fn push_merged(inlines: &mut Vec<Inline>, inline: Inline) {
    if let Inline::Text {
        text,
        emphasis,
        strong,
        strike,
    } = &inline
        && let Some(Inline::Text {
            text: prev,
            emphasis: pe,
            strong: ps,
            strike: pk,
        }) = inlines.last_mut()
        && pe == emphasis
        && ps == strong
        && pk == strike
    {
        prev.push_str(text);
        return;
    }
    inlines.push(inline);
}

/// The text of inlines without markup, for slugs, titles and image descriptions.
fn plain_text(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text { text, .. } | Inline::Code { text } => out.push_str(text),
            Inline::Link { inlines, .. } => out.push_str(&plain_text(inlines)),
            Inline::Math { latex } => out.push_str(latex),
            Inline::Image { alt, .. } => out.push_str(alt),
            Inline::LineBreak => out.push(' '),
            Inline::FootnoteRef { .. } | Inline::Status { .. } | Inline::AnchorMark { .. } => {}
        }
    }
    out
}

/// A heading id the way GitHub builds it: lowercase, spaces to hyphens, punctuation dropped.
fn slugify(text: &str) -> String {
    let mut slug = String::new();
    for c in text.trim().chars() {
        if c.is_alphanumeric() {
            slug.extend(c.to_lowercase());
        } else if c == ' ' || c == '-' {
            slug.push('-');
        } else if c == '_' {
            slug.push('_');
        }
    }
    slug
}

/// True for a target inside this document or a relative path; false for any URL with a scheme.
fn is_internal(target: &str) -> bool {
    if target.is_empty() {
        return false;
    }
    if target.starts_with('#') {
        return true;
    }
    if target.starts_with("//") {
        return false;
    }
    let before = target.split(['/', '?', '#']).next().unwrap_or("");
    let has_scheme = before.split_once(':').is_some_and(|(scheme, _)| {
        scheme.len() >= 2
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    });
    !has_scheme
}

fn strip_one_newline(mut text: String) -> String {
    if text.ends_with('\n') {
        text.pop();
    }
    text
}
