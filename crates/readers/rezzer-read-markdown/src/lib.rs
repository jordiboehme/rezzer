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
    Alignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag,
    TagEnd,
};
use rezzer_limits::{Budget, DepthGuard};
use rezzer_model::{
    Align, Anchor, Block, CalloutKind, Cell, Completeness, Document, Edge, Embedded, FormatId,
    Graph, Inline, Loss, LossReason, Node, ReadOptions, ReadOutcome, Reader, ReaderInfo,
    SectionKind, Severity, Source, TaskState, plain_text, source_info,
};

use frontmatter::{FmError, Syntax};

/// How many parser events are handled between two time checks.
const CHECK_EVERY: usize = 1024;

/// Parser tree nodes allowed per model element of the budget. Markup bytes (see
/// [`parser_cut`]) stand for the nodes the parser builds.
const PARSER_NODES_PER_ELEMENT: u64 = 2;

/// Frontmatter over this size is not parsed.
const MAX_FRONTMATTER_BYTES: usize = 1024 * 1024;

/// Frontmatter over this size is kept as text but not parsed. Real frontmatter is tiny. The
/// parser and the writing of the properties are bounded by depth, work and time budgets; this
/// cap is defense in depth.
const MAX_PARSE_BYTES: usize = 256 * 1024;

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
            blobs: Vec::new(),
            document: doc,
            completeness,
        }
    }
}

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_MATH
        | Options::ENABLE_WIKILINKS
        | Options::ENABLE_GFM
        | Options::ENABLE_STRIKETHROUGH
}

/// An open container block.
enum Frame<'b> {
    Root {
        blocks: Vec<Block>,
    },
    Quote {
        kind: Option<BlockQuoteKind>,
        blocks: Vec<Block>,
        /// Index of the first line break inside the quote's first paragraph, kept to read a
        /// callout title (`> [!NOTE] Title`) apart from the body.
        callout_break: Option<usize>,
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
}

/// An open link or image inside a leaf block.
enum InlineFrame {
    Link {
        target: String,
        title: Option<String>,
        internal: bool,
        wiki: bool,
        inlines: Vec<Inline>,
    },
    Image {
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
    alignments: Vec<Align>,
}

struct Builder<'b> {
    budget: &'b Budget,
    stack: Vec<Frame<'b>>,
    leaf: Leaf,
    cur: Vec<Inline>,
    inline_stack: Vec<InlineFrame>,
    emphasis: u32,
    strong: u32,
    strike: u32,
    /// True when the current inlines hold a display formula from `$$ ... $$`.
    display_math: bool,
    table: Option<TableState>,
    /// Nesting counter while a subtree over the depth limit is skipped.
    skip: usize,
    skipped_subtrees: usize,
    /// Index in `losses` of the depth loss, for the count of skipped places.
    depth_loss: Option<usize>,
    /// Byte offset of the event being handled.
    offset: usize,
    /// Set when the depth loss still needs its line number.
    need_line: bool,
    /// Index in `losses` of the empty-math loss and how many empty formulas were dropped.
    empty_math_loss: Option<usize>,
    empty_math: usize,
    /// Set when the empty-math loss still needs its line number.
    need_math_line: bool,
    /// Index in `cur` of the first soft break of a possible callout header.
    para_break: Option<usize>,
    time_stopped: bool,
    /// Set when the element budget ran out; the event loop then stops.
    elements_stopped: bool,
    /// The line where the parser input was cut to bound the parser's own tree.
    input_cut: Option<usize>,
    footnotes: Vec<(String, Vec<Block>)>,
    heading_slugs: HashMap<String, u32>,
    first_h1: Option<String>,
    properties: rezzer_model::Properties,
    losses: Vec<Loss>,
    nodes: Vec<Node>,
    node_ids: HashMap<String, usize>,
    edges: Vec<Edge>,
    edge_keys: HashSet<(String, &'static str)>,
    embedded: Vec<Embedded>,
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
            strike: 0,
            display_math: false,
            table: None,
            skip: 0,
            skipped_subtrees: 0,
            depth_loss: None,
            offset: 0,
            need_line: false,
            empty_math_loss: None,
            empty_math: 0,
            need_math_line: false,
            para_break: None,
            time_stopped: false,
            elements_stopped: false,
            input_cut: None,
            footnotes: Vec::new(),
            heading_slugs: HashMap::new(),
            first_h1: None,
            properties: rezzer_model::Properties::default(),
            losses: Vec::new(),
            nodes: Vec::new(),
            node_ids: HashMap::new(),
            edges: Vec::new(),
            edge_keys: HashSet::new(),
            embedded: Vec::new(),
            body: Vec::new(),
        }
    }

    fn run(&mut self, text: &str) {
        // Frontmatter is read only at the very start; the parser sees the rest as Markdown.
        let body_start = match frontmatter::split(text) {
            Some(fm) => {
                self.frontmatter(fm.syntax, fm.raw);
                fm.body_start
            }
            None => 0,
        };
        let body = fix_empty_potholes(&text[body_start..]);
        // The parser builds a tree node for every run of inline markup in the whole input
        // before it gives the first event, so the input it gets is bounded too.
        let cap = self
            .budget
            .limits()
            .max_elements
            .saturating_mul(PARSER_NODES_PER_ELEMENT);
        let parsed: &str = match parser_cut(&body, cap) {
            Some(at) => {
                let line =
                    text[..body_start].matches('\n').count() + body[..at].matches('\n').count() + 1;
                self.input_cut = Some(line);
                &body[..at]
            }
            None => &body,
        };
        let parser = Parser::new_ext(parsed, options());
        for (n, (event, range)) in parser.into_offset_iter().enumerate() {
            if n % CHECK_EVERY == CHECK_EVERY - 1 && self.budget.check_time().is_err() {
                self.time_stopped = true;
                break;
            }
            self.offset = body_start + range.start;
            self.event(event);
            if self.elements_stopped {
                break;
            }
            if self.need_math_line {
                self.need_math_line = false;
                let end = self.offset.min(text.len());
                let line = text.as_bytes()[..end]
                    .iter()
                    .filter(|&&b| b == b'\n')
                    .count()
                    + 1;
                if let Some(loss) = self.empty_math_loss.and_then(|i| self.losses.get_mut(i)) {
                    loss.location = format!("line {line}");
                }
            }
            if self.need_line {
                self.need_line = false;
                let end = self.offset.min(text.len());
                let line = text.as_bytes()[..end]
                    .iter()
                    .filter(|&&b| b == b'\n')
                    .count()
                    + 1;
                if let Some(loss) = self.depth_loss.and_then(|i| self.losses.get_mut(i)) {
                    loss.location = format!("line {line}");
                }
            }
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
                // Rezzer writes inline math as $`...`$; the backticks are not part of the formula.
                let latex = match s.strip_prefix('`').and_then(|t| t.strip_suffix('`')) {
                    Some(inner) => inner.to_string(),
                    None => s.into_string(),
                };
                self.math_or_loss(latex);
            }
            Event::DisplayMath(s) => {
                self.ensure_inline_leaf();
                if s.trim().is_empty() {
                    self.math_or_loss(s.into_string());
                    return;
                }
                // Only a formula that is the whole paragraph is a display block.
                let alone = self.cur.is_empty() && self.inline_stack.is_empty();
                self.push_inline(Inline::Math {
                    latex: s.into_string(),
                });
                self.display_math = alone;
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
            Event::SoftBreak => self.soft_break(),
            Event::HardBreak => {
                self.ensure_inline_leaf();
                if self.can_mark_callout_break() {
                    self.para_break = Some(self.cur.len());
                    self.cur.push(Inline::LineBreak);
                } else {
                    self.push_inline(Inline::LineBreak);
                }
            }
            Event::Rule => {
                self.flush_implicit();
                self.push_block(Block::Rule { anchor: None });
            }
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
                self.display_math = false;
                self.para_break = None;
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
                        callout_break: None,
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
            Tag::Table(aligns) => {
                self.flush_implicit();
                let mut state = TableState::default();
                if aligns.iter().any(|a| !matches!(a, Alignment::None)) {
                    state.alignments = aligns
                        .iter()
                        .map(|a| match a {
                            Alignment::None => Align::None,
                            Alignment::Left => Align::Left,
                            Alignment::Center => Align::Center,
                            Alignment::Right => Align::Right,
                        })
                        .collect();
                }
                self.table = Some(state);
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
            Tag::Strikethrough => {
                self.ensure_inline_leaf();
                self.strike += 1;
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
                    wiki: wikilink,
                    inlines: Vec::new(),
                });
            }
            Tag::Image {
                dest_url, title, ..
            } => {
                self.ensure_inline_leaf();
                self.inline_stack.push(InlineFrame::Image {
                    target: dest_url.into_string(),
                    title: title.into_string(),
                    alt: String::new(),
                });
            }
            // Frontmatter is split off before parsing, so the parser never reports it.
            Tag::MetadataBlock(_) => {}
            // Not enabled in `options()`; kept as plain text if they ever appear.
            Tag::Superscript
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
                        role: None,
                        rows: t.rows,
                        alignments: t.alignments,
                        anchor: None,
                        layout: None,
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
                self.display_math = false;
                if self.charge()
                    && let Some(t) = &mut self.table
                {
                    t.row.push(Cell::new(inlines));
                }
            }
            TagEnd::Emphasis => self.emphasis = self.emphasis.saturating_sub(1),
            TagEnd::Strong => self.strong = self.strong.saturating_sub(1),
            TagEnd::Link | TagEnd::Image => self.end_inline_frame(),
            TagEnd::MetadataBlock(_) => {}
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Superscript
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
                    self.depth_loss = Some(self.losses.len());
                    self.need_line = true;
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
            Leaf::Code { text, .. } | Leaf::Html(text) => {
                text.push_str(s);
            }
            _ => {
                self.ensure_inline_leaf();
                self.push_inline(Inline::Text {
                    text: s.to_string(),
                    emphasis: self.emphasis > 0,
                    strong: self.strong > 0,
                    strike: self.strike > 0,
                });
            }
        }
    }

    /// True in the first paragraph of a plain quote that starts like a callout header, before
    /// its first line break.
    fn can_mark_callout_break(&self) -> bool {
        matches!(self.leaf, Leaf::Paragraph)
            && matches!(
                self.stack.last(),
                Some(Frame::Quote { kind: None, blocks, .. }) if blocks.is_empty()
            )
            && matches!(
                self.cur.first(),
                Some(Inline::Text { text, .. }) if text.starts_with("[!")
            )
            && self.para_break.is_none()
            && self.inline_stack.is_empty()
    }

    /// A soft break reads as a space. In the first paragraph of a plain quote its position is
    /// kept as a separate inline, so a callout header (`> [!NOTE] Title`) can be told from the
    /// body; `close_top_frame` joins it again when the quote is no callout.
    fn soft_break(&mut self) {
        self.ensure_inline_leaf();
        let space = Inline::plain(" ");
        if self.can_mark_callout_break() {
            self.para_break = Some(self.cur.len());
            self.cur.push(space);
        } else {
            self.push_inline(Inline::Text {
                text: " ".into(),
                emphasis: self.emphasis > 0,
                strong: self.strong > 0,
                strike: self.strike > 0,
            });
        }
    }

    fn ensure_inline_leaf(&mut self) {
        if matches!(self.leaf, Leaf::None) {
            self.leaf = Leaf::Implicit;
        }
    }

    /// Charges one model element. False when the element budget ran out with this one: the
    /// element is dropped and reading stops. Once stopped, the frames still open are closed
    /// without charge (their number is bounded by the depth limit).
    fn charge(&mut self) -> bool {
        if self.elements_stopped {
            return true;
        }
        if self.budget.add_elements(1).is_err() {
            self.elements_stopped = true;
            return false;
        }
        true
    }

    /// Pushes a formula. An empty one has nothing to show and is not kept, but it is said:
    /// the source had math delimiters with nothing inside.
    fn math_or_loss(&mut self, latex: String) {
        if !latex.trim().is_empty() {
            self.push_inline(Inline::Math { latex });
            return;
        }
        self.empty_math += 1;
        if self.empty_math == 1 {
            self.empty_math_loss = Some(self.losses.len());
            self.need_math_line = true;
            self.losses.push(Loss {
                location: "document".into(),
                what: "empty math (a formula with no content) dropped".into(),
                why: LossReason::Uncertain,
                severity: Severity::Warning,
            });
        }
    }

    fn push_inline(&mut self, inline: Inline) {
        if !self.charge() {
            return;
        }
        self.display_math = false;
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
                wiki,
                inlines,
            }) => {
                self.record_link(&target, internal);
                self.push_inline(Inline::Link {
                    target,
                    title,
                    internal,
                    wiki,
                    inlines,
                });
            }
            Some(InlineFrame::Image { target, title, alt }) => {
                self.record_image(&target, &alt);
                self.push_inline(Inline::Image {
                    target,
                    alt,
                    title: Some(title).filter(|t| !t.is_empty()),
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

    /// Reads the frontmatter into the properties. What cannot be read stays visible as a code
    /// block at the top of the body, together with a loss.
    fn frontmatter(&mut self, syntax: Syntax, raw: &str) {
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

    // --- container blocks -----------------------------------------------------------------

    fn push_block(&mut self, block: Block) {
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

    fn close_top_frame(&mut self) {
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

    // --- link graph -----------------------------------------------------------------------

    /// Adds a node for `target` unless it exists and returns its id. A fragment-only target
    /// (`#hello`) gets the id of the heading anchor it points to (`h:hello`).
    fn node_for(&mut self, target: &str, internal: bool, image: bool) -> String {
        // "self" is the document node, so a target that is literally "self" gets another id.
        let (id, anchor) = match target.strip_prefix('#') {
            Some(frag) if !frag.is_empty() => {
                let id = if frag.starts_with("h:") {
                    frag.to_string()
                } else {
                    // The same slug as the heading anchors, so `#Hello-World` and
                    // `#hello%20world` both point to `h:hello-world`.
                    let slug = slugify(&percent_decode(frag));
                    if slug.is_empty() {
                        format!("h:{frag}")
                    } else {
                        format!("h:{slug}")
                    }
                };
                let anchor = Some(Anchor(id.clone()));
                (id, anchor)
            }
            _ if target == "self" => ("link:self".to_string(), None),
            _ => (target.to_string(), None),
        };
        if let Some(&at) = self.node_ids.get(&id) {
            // A target that is also embedded as a picture counts as a picture.
            if image {
                self.nodes[at].kind = Some("image".into());
            }
            return id;
        }
        let kind = if image {
            "image"
        } else if target.starts_with('#') {
            "anchor"
        } else if internal {
            "path"
        } else {
            "url"
        };
        self.node_ids.insert(id.clone(), self.nodes.len());
        self.nodes.push(Node {
            id: id.clone(),
            label: target.to_string(),
            kind: Some(kind.into()),
            anchor,
        });
        id
    }

    fn add_edge(&mut self, id: String, kind: &'static str) {
        if self.edge_keys.insert((id.clone(), kind)) {
            self.edges.push(Edge {
                from: "self".into(),
                to: id,
                kind: kind.into(),
                anchor: None,
            });
        }
    }

    fn record_link(&mut self, target: &str, internal: bool) {
        if target.is_empty() {
            return;
        }
        let id = self.node_for(target, internal, false);
        self.add_edge(id, "links_to");
    }

    /// An image is a relation to another file and an embedded reference; it is not fetched.
    fn record_image(&mut self, target: &str, alt: &str) {
        if target.is_empty() {
            return;
        }
        let id = self.node_for(target, is_internal(target), true);
        self.add_edge(id, "embeds");
        let name = if alt.trim().is_empty() {
            target
                .split(['?', '#'])
                .next()
                .and_then(|p| p.rsplit('/').next())
                .filter(|n| !n.is_empty())
                .unwrap_or(target)
                .to_string()
        } else {
            alt.to_string()
        };
        self.embedded.push(Embedded {
            text: None,
            kind: "image".into(),
            name,
            detail: None,
            target: Some(target.to_string()),
        });
    }

    /// Moves the collected properties, losses and body into the document.
    fn finish_into(&mut self, doc: &mut Document) {
        doc.properties = std::mem::take(&mut self.properties);
        doc.losses.append(&mut self.losses);
        doc.body = std::mem::take(&mut self.body);
        doc.facets.embedded = std::mem::take(&mut self.embedded);
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

/// The byte offset where `body` is cut so that the parser sees at most `cap` markup bytes,
/// the bytes that can start an inline or block node (emphasis, code, links, HTML, entities,
/// escapes, strike, math, table pipes and line ends). None when the whole body fits.
fn parser_cut(body: &str, cap: u64) -> Option<usize> {
    let mut seen = 0u64;
    for (i, b) in body.bytes().enumerate() {
        if matches!(
            b,
            b'*' | b'_'
                | b'`'
                | b'['
                | b']'
                | b'!'
                | b'<'
                | b'&'
                | b'\\'
                | b'~'
                | b'$'
                | b'|'
                | b'\n'
        ) {
            seen += 1;
            if seen > cap {
                // Markup bytes are ASCII, so `i` is a char boundary.
                return Some(i);
            }
        }
    }
    None
}

/// pulldown-cmark 0.13.4 repeats the rest of the paragraph and loses the next wikilink's
/// target after an empty pothole `[[x|]]`. Written as `[[x]]` it means the same. Code spans,
/// fenced and indented code stay as they are.
fn fix_empty_potholes(body: &str) -> std::borrow::Cow<'_, str> {
    if !body.contains("|]]") {
        return std::borrow::Cow::Borrowed(body);
    }
    let mut out = String::with_capacity(body.len());
    let mut fence: Option<char> = None;
    for line in body.split_inclusive('\n') {
        let t = line.trim_start_matches(' ');
        let marker = t.chars().next().filter(|c| matches!(c, '`' | '~'));
        let is_fence = marker.is_some_and(|m| t.starts_with(&m.to_string().repeat(3)));
        if let Some(open) = fence {
            if is_fence && marker == Some(open) {
                fence = None;
            }
            out.push_str(line);
            continue;
        }
        if is_fence {
            fence = marker;
            out.push_str(line);
            continue;
        }
        if line.starts_with("    ") || line.starts_with('\t') {
            out.push_str(line);
            continue;
        }
        let mut rest = line;
        let mut ticks = 0usize;
        while let Some(pos) = rest.find("|]]") {
            let before = &rest[..pos];
            ticks += before.matches('`').count();
            let open = before.rfind("[[");
            let plain_target = open.is_some_and(|o| {
                let target = &before[o + 2..];
                !target.is_empty() && !target.contains(['[', ']', '|', '`'])
            });
            out.push_str(before);
            if !(plain_target && ticks.is_multiple_of(2)) {
                out.push('|');
            }
            out.push_str("]]");
            rest = &rest[pos + 3..];
        }
        out.push_str(rest);
    }
    std::borrow::Cow::Owned(out)
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

/// Decodes `%XX` escapes; invalid UTF-8 after decoding becomes U+FFFD, a `%` without two hex
/// digits stays as it is.
fn percent_decode(text: &str) -> String {
    if !text.contains('%') {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(hi), Some(lo)) = (
                bytes.get(i + 1).and_then(|&b| hex(b)),
                bytes.get(i + 2).and_then(|&b| hex(b)),
            )
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A heading id the way GitHub builds it: lowercase, spaces to hyphens, punctuation dropped.
fn slugify(text: &str) -> String {
    let mut slug = String::new();
    for c in strip_html_tags(text).trim().chars() {
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

/// Removes `<tag>`, `</tag>` and `<!-- -->` spans; a lone `<` followed by a space stays.
fn strip_html_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let is_tag = tail[1..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '/' || c == '!');
        match tail.find('>') {
            Some(j) if is_tag => rest = &tail[j + 1..],
            _ => {
                out.push('<');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn strip_one_newline(mut text: String) -> String {
    if text.ends_with('\n') {
        text.pop();
    }
    text
}
