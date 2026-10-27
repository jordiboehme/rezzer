// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The builder that turns parser events into the document model: frames, the event loop and the
//! start and end of each element.

use std::collections::{HashMap, HashSet};

use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag,
    TagEnd,
};
use rezzer_limits::{Budget, DepthGuard};
use rezzer_model::{
    Align, Block, Cell, Edge, Embedded, Inline, Loss, LossReason, Node, Severity, TaskState,
};

use crate::frontmatter;

mod blocks;
mod graph;
mod inlines;

use blocks::strip_one_newline;
use graph::is_internal;

/// How many parser events are handled between two time checks.
const CHECK_EVERY: usize = 1024;

/// Parser tree nodes allowed per model element of the budget. Markup bytes (see
/// [`parser_cut`]) stand for the nodes the parser builds.
const PARSER_NODES_PER_ELEMENT: u64 = 2;

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

pub(super) struct Builder<'b> {
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
    pub(super) first_h1: Option<String>,
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
    pub(super) fn new(budget: &'b Budget) -> Self {
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

    pub(super) fn run(&mut self, text: &str) {
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

/// An empty pothole `[[x|]]` is written as `[[x]]`, which means the same. pulldown-cmark
/// 0.13.4 repeated the rest of the paragraph and lost the next wikilink's target after one;
/// 0.13.5 reads it, but as a link with no text. Code spans, fenced and indented code stay as
/// they are.
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
