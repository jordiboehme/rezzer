// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Body blocks and inlines as Markdown in Crystalline's dialect.
//!
//! Text from files is escaped so that it can never add structure: no headings, lists, quotes,
//! tables, code, HTML (and so no HTML comments), math or frontmatter. Anchors are not printed:
//! the model JSON keeps them, and a heading anchor is the heading's own slug anyway. The one
//! exception is a sheet section, whose cell lines end with their cell's address (`layout`).

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;

use rezzer_model::{
    Align, Annotation, Asset, Barcode, Block, CalloutKind, Cell, Inline, SectionKind, StatusColor,
    TaskState, column_letter, plain_text, sheet_cell,
};

mod inline;
mod links;
mod table;

pub(crate) use inline::*;
pub(crate) use links::*;
pub(crate) use table::*;

/// Deeper nesting than this is cut with a visible note, so a hostile file cannot overflow
/// the stack.
pub(crate) const MAX_DEPTH: usize = 64;

const TOO_DEEP: &str = "Nesting too deep: the rest of this part is not shown.";

/// Where inline text goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// A paragraph: newlines stay.
    Block,
    /// One line: a heading, a title, link text.
    Line,
    /// One line inside a table cell: `|` is escaped even inside code.
    Cell,
}

/// What the block renderer needs besides the blocks.
pub(crate) struct Ctx<'a> {
    pub assets: &'a [Asset],
    /// Put before every asset path in a link: `../` in a part file one folder down, else
    /// empty.
    pub link_base: &'a str,
    /// Added to every body heading level, so body headings stay below the title.
    pub heading_offset: u8,
    /// Annotations to show at their passage; only the content has them.
    pub notes: Option<&'a Notes<'a>>,
    /// The assets whose codes were written already: a code is written once, at the first
    /// place of its picture.
    pub codes_shown: RefCell<std::collections::HashSet<String>>,
    /// How many sheet sections the renderer is inside. Blocks of a sheet take no inline
    /// notes: a cell's comment stays in `## Annotations` even where the cell is a caption,
    /// a heading or a one-cell region.
    pub in_sheet: std::cell::Cell<usize>,
    /// How the rows of a sheet table are written.
    pub table_style: crate::TableStyle,
    /// A table column without a header is named by its number (`Column 4`, CSV and TSV),
    /// else by its sheet column letter (`Column D`).
    pub column_numbers: bool,
}

/// The annotations of a document by the anchor they stand at, and which of them the content
/// showed. Each one is shown once: after the first block with its anchor that is rendered, or
/// at the end of its page section; never inside a sheet section. The ones not shown stay for
/// `## Annotations`.
pub(crate) struct Notes<'a> {
    list: &'a [Annotation],
    at: HashMap<&'a str, Vec<usize>>,
    shown: RefCell<Vec<bool>>,
    /// The annotations shown, in the order they were shown, so a trial render can be undone.
    taken: RefCell<Vec<usize>>,
}

impl<'a> Notes<'a> {
    pub(crate) fn new(list: &'a [Annotation]) -> Notes<'a> {
        let mut at: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, a) in list.iter().enumerate() {
            if let Some(anchor) = &a.anchor {
                at.entry(anchor.0.as_str()).or_default().push(i);
            }
        }
        Notes {
            list,
            at,
            shown: RefCell::new(vec![false; list.len()]),
            taken: RefCell::new(Vec::new()),
        }
    }

    /// A mark to go back to with [`Notes::rollback`].
    pub(crate) fn checkpoint(&self) -> usize {
        self.taken.borrow().len()
    }

    /// Forgets every annotation shown since `mark`: the splitter renders a part to measure
    /// it, then renders it again for real. Costs only what was shown since.
    pub(crate) fn rollback(&self, mark: usize) {
        let mut taken = self.taken.borrow_mut();
        let mut shown = self.shown.borrow_mut();
        for i in taken.drain(mark..) {
            shown[i] = false;
        }
    }

    /// True when annotation `i` was shown at its passage.
    pub(crate) fn shown(&self, i: usize) -> bool {
        self.shown.borrow().get(i).copied().unwrap_or(false)
    }

    /// The annotations at `anchor` not shown yet, now marked as shown.
    fn take(&self, anchor: &str) -> Vec<&'a Annotation> {
        let Some(list) = self.at.get(anchor) else {
            return Vec::new();
        };
        let mut shown = self.shown.borrow_mut();
        let mut taken = self.taken.borrow_mut();
        list.iter()
            .filter(|&&i| !std::mem::replace(&mut shown[i], true))
            .map(|&i| {
                taken.push(i);
                &self.list[i]
            })
            .collect()
    }
}

/// The nesting levels a note at a passage may take below that passage.
const NOTE_DEPTH: usize = 8;

/// The anchor a block's annotations stand at, for every block but sections.
fn passage_anchor(b: &Block) -> Option<&str> {
    match b {
        Block::Section { .. } => None,
        Block::Heading { anchor, .. }
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

/// `text` with the note callouts of the annotations at `anchor` after it.
fn with_notes(text: String, anchor: Option<&str>, ctx: &Ctx, depth: usize, level: u8) -> String {
    let (Some(notes), Some(anchor)) = (ctx.notes, anchor) else {
        return text;
    };
    if ctx.in_sheet.get() > 0 {
        return text;
    }
    // A note needs a few levels of its own (callout, quote, reply lists); too deep a passage
    // leaves its notes to `## Annotations`, where they are shown whole.
    if depth + NOTE_DEPTH > MAX_DEPTH {
        return text;
    }
    let mut out = text;
    for a in notes.take(anchor) {
        let note = block(&crate::facets::passage_note(a), ctx, depth + 1, level);
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&note);
    }
    out
}

// ---------------------------------------------------------------------------------------
// Inlines

pub(crate) fn status_color(color: StatusColor) -> &'static str {
    match color {
        StatusColor::Grey => "grey",
        StatusColor::Blue => "blue",
        StatusColor::Green => "green",
        StatusColor::Yellow => "yellow",
        StatusColor::Red => "red",
        StatusColor::Purple => "purple",
    }
}

/// Makes a rendered paragraph safe line by line: blank lines go (they would end the
/// paragraph), the first line loses its indent (it would become code) and every line that
/// starts with a block marker gets it escaped.
pub(crate) fn finish_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut first = true;
    for line in text.split('\n') {
        if line.trim().is_empty() {
            continue;
        }
        if !first {
            out.push('\n');
        }
        let line = if first { line.trim_start() } else { line };
        first = false;
        let indent = line.len() - line.trim_start().len();
        out.push_str(&line[..indent]);
        escape_line_start(&line[indent..], &mut out);
    }
    out
}

/// Writes `line`, a block marker at its start escaped (`#`, `>`, `-`, `+`, `=`, `1.`).
pub(crate) fn escape_line_start(line: &str, out: &mut String) {
    match line.chars().next() {
        Some('#' | '>' | '-' | '+' | '=') => {
            out.push('\\');
            out.push_str(line);
        }
        Some(c) if c.is_ascii_digit() => {
            let digits = line.chars().take_while(char::is_ascii_digit).count();
            match line[digits..].chars().next() {
                Some('.' | ')') => {
                    out.push_str(&line[..digits]);
                    out.push('\\');
                    out.push_str(&line[digits..]);
                }
                _ => out.push_str(line),
            }
        }
        _ => out.push_str(line),
    }
}

/// A paragraph from inlines; empty when there is no visible content.
pub(crate) fn paragraph(list: &[Inline]) -> String {
    let start = list
        .iter()
        .position(|i| !matches!(i, Inline::LineBreak))
        .unwrap_or(list.len());
    let end = list
        .iter()
        .rposition(|i| !matches!(i, Inline::LineBreak))
        .map_or(start, |e| e + 1);
    finish_lines(&inlines(&list[start..end.max(start)], Mode::Block))
}

// ---------------------------------------------------------------------------------------
// Blocks

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrevKind {
    None,
    Bullet,
    Ordered,
    Other,
}

#[derive(Debug, Clone, Copy)]
struct Prev {
    kind: PrevKind,
    alt: bool,
    task: bool,
    /// A paragraph, list or task: a following list may join it tightly.
    tight_lead: bool,
}

/// Renders a block sequence. `level` is the heading level for titled sections; `tight`
/// joins a paragraph and a following list without a blank line (inside list items).
pub(crate) fn blocks(list: &[Block], ctx: &Ctx, depth: usize, level: u8, tight: bool) -> String {
    if depth > MAX_DEPTH {
        return TOO_DEEP.into();
    }
    let mut out = String::new();
    let mut prev = Prev {
        kind: PrevKind::None,
        alt: false,
        task: false,
        tight_lead: false,
    };
    for item in list {
        let (text, now) = match item {
            Block::Task { state, inlines, .. } => {
                let alt = if prev.task {
                    prev.alt
                } else {
                    prev.kind == PrevKind::Bullet && !prev.alt
                };
                (
                    with_notes(
                        task(*state, inlines, alt),
                        passage_anchor(item),
                        ctx,
                        depth,
                        level,
                    ),
                    Prev {
                        kind: PrevKind::Bullet,
                        alt,
                        task: true,
                        tight_lead: true,
                    },
                )
            }
            Block::List { ordered, items, .. } => {
                let same = if *ordered {
                    PrevKind::Ordered
                } else {
                    PrevKind::Bullet
                };
                let alt = prev.kind == same && !prev.alt;
                (
                    with_notes(
                        list_block(*ordered, items, alt, ctx, depth, level),
                        passage_anchor(item),
                        ctx,
                        depth,
                        level,
                    ),
                    Prev {
                        kind: same,
                        alt,
                        task: false,
                        tight_lead: true,
                    },
                )
            }
            other => (
                with_notes(
                    block(other, ctx, depth, level),
                    passage_anchor(other),
                    ctx,
                    depth,
                    level,
                ),
                Prev {
                    kind: PrevKind::Other,
                    alt: false,
                    task: false,
                    tight_lead: matches!(other, Block::Paragraph { .. }),
                },
            ),
        };
        if text.is_empty() {
            continue;
        }
        if !out.is_empty() {
            let joins_tight = (prev.task && now.task)
                || (tight && prev.tight_lead && now.kind != PrevKind::Other);
            out.push_str(if joins_tight { "\n" } else { "\n\n" });
        }
        if out.is_empty() {
            out = text;
        } else {
            out.push_str(&text);
        }
        prev = now;
    }
    out
}

fn block(b: &Block, ctx: &Ctx, depth: usize, level: u8) -> String {
    match b {
        Block::Section {
            kind,
            title,
            anchor,
            blocks: inner,
        } => {
            let label = anchor.as_ref().and_then(|a| a.0.strip_prefix("fn:"));
            let title = title.as_deref().filter(|t| !t.trim().is_empty());
            if let (None, Some(label)) = (title, label) {
                return footnote(label, inner, ctx, depth);
            }
            let sheet = *kind == SectionKind::Sheet;
            section(
                *kind,
                title,
                anchor.as_ref().map(|a| a.0.as_str()),
                ctx,
                depth,
                level,
                &|d, l| section_body(inner, sheet, ctx, d, l),
            )
        }
        Block::Heading {
            level: l,
            inlines: content,
            ..
        } => {
            let l = l.saturating_add(ctx.heading_offset).clamp(1, 6);
            heading_line(l, inlines(content, Mode::Line).trim())
        }
        Block::Paragraph {
            inlines: content, ..
        } => paragraph(content),
        Block::List { ordered, items, .. } => list_block(*ordered, items, false, ctx, depth, level),
        Block::Table {
            caption,
            header_rows,
            rows,
            alignments,
            layout,
            ..
        } => match layout {
            Some(layout) => crate::layout::layout_table(
                layout,
                *header_rows,
                rows,
                alignments,
                true,
                ctx,
                depth,
                level,
            ),
            None => model_table(caption.as_deref(), *header_rows, rows, alignments),
        },
        Block::Code { lang, text, .. } => fence(lang.as_deref(), text),
        Block::Quote { blocks: inner, .. } => {
            let body = blocks(inner, ctx, depth + 1, level, false);
            prefix_lines(&body, ">")
        }
        Block::Math { latex, .. } => math_block(latex),
        Block::Media { asset, alt, .. } => media(asset, alt.as_deref(), ctx),
        Block::Callout {
            kind,
            title,
            folded,
            blocks: inner,
            ..
        } => {
            let mut head = format!("[!{}]", callout_kind(*kind));
            match folded {
                Some(true) => head.push('-'),
                Some(false) => head.push('+'),
                None => {}
            }
            if let Some(title) = title.as_deref().filter(|t| !t.trim().is_empty()) {
                head.push(' ');
                head.push_str(&line_text(title.trim()));
            }
            let body = blocks(inner, ctx, depth + 1, level, false);
            prefix_lines(&join_lines(head, body), ">")
        }
        Block::Rule { .. } => "---".into(),
        Block::Task { state, inlines, .. } => task(*state, inlines, false),
    }
}

/// A section with `title` (none or blank: no heading) around the text `body` makes at the
/// depth and heading level it is given. A sheet's blocks take no inline notes; a page's
/// annotations that stand at no block (at `anchor`) close it. The splitter renders a part of a
/// section's blocks this way, so the part keeps the section's heading and context.
pub(crate) fn section(
    kind: SectionKind,
    title: Option<&str>,
    anchor: Option<&str>,
    ctx: &Ctx,
    depth: usize,
    level: u8,
    body: &dyn Fn(usize, u8) -> String,
) -> String {
    let sheet = kind == SectionKind::Sheet;
    if sheet {
        ctx.in_sheet.set(ctx.in_sheet.get() + 1);
    }
    let text = match title.filter(|t| !t.trim().is_empty()) {
        Some(title) => {
            let head = heading_line(level.min(6), &line_text(title.trim()));
            join2(head, body(depth + 1, level.saturating_add(1)))
        }
        None => body(depth + 1, level),
    };
    if sheet {
        ctx.in_sheet.set(ctx.in_sheet.get() - 1);
    }
    // The annotations of a page that stand at no block close its section.
    let page = (kind == SectionKind::Page).then_some(anchor).flatten();
    with_notes(text, page, ctx, depth, level)
}

/// The blocks of a section: a sheet's through the sheet layout, others as a block sequence.
pub(crate) fn section_body(
    list: &[Block],
    sheet: bool,
    ctx: &Ctx,
    depth: usize,
    level: u8,
) -> String {
    if sheet {
        crate::layout::sheet_blocks(list, ctx, depth, level)
    } else {
        blocks(list, ctx, depth, level, false)
    }
}

fn join2(a: String, b: String) -> String {
    if b.is_empty() {
        a
    } else {
        format!("{a}\n\n{b}")
    }
}

fn join_lines(a: String, b: String) -> String {
    if b.is_empty() { a } else { format!("{a}\n{b}") }
}

/// `#` times `level`, then the text with a trailing `#` run escaped (it would close the
/// heading).
pub(crate) fn heading_line(level: u8, text: &str) -> String {
    let hashes = "#".repeat(usize::from(level.max(1)));
    if text.is_empty() {
        return hashes;
    }
    let trimmed = text.trim_end_matches('#');
    if trimmed.len() == text.len() {
        format!("{hashes} {text}")
    } else {
        format!("{hashes} {trimmed}\\{}", &text[trimmed.len()..])
    }
}

fn callout_kind(kind: CalloutKind) -> &'static str {
    match kind {
        CalloutKind::Note => "NOTE",
        CalloutKind::Tip => "TIP",
        CalloutKind::Important => "IMPORTANT",
        CalloutKind::Warning => "WARNING",
        CalloutKind::Caution => "CAUTION",
        CalloutKind::Details => "DETAILS",
    }
}

/// Prefixes every line with `marker` (`>`), with a space before non-empty lines.
fn prefix_lines(text: &str, marker: &str) -> String {
    if text.is_empty() {
        return marker.into();
    }
    text.split('\n')
        .map(|l| {
            if l.is_empty() {
                marker.to_string()
            } else {
                format!("{marker} {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The first line after `first`, the rest indented by `indent` spaces.
fn hang(first: &str, body: &str, indent: usize) -> String {
    if body.is_empty() {
        return first.trim_end().to_string();
    }
    let pad = " ".repeat(indent);
    let mut out = String::new();
    for (i, line) in body.split('\n').enumerate() {
        if i == 0 {
            out.push_str(first);
            out.push_str(line);
        } else {
            out.push('\n');
            if !line.is_empty() {
                out.push_str(&pad);
                out.push_str(line);
            }
        }
    }
    out
}

pub(crate) fn list_block(
    ordered: bool,
    items: &[Vec<Block>],
    alt: bool,
    ctx: &Ctx,
    depth: usize,
    level: u8,
) -> String {
    if depth > MAX_DEPTH {
        return TOO_DEEP.into();
    }
    // Loose when an item has blocks that need a blank line between them; then every item
    // keeps blank lines, also before a nested list.
    let loose = items.iter().any(|item| {
        item.windows(2).any(|w| {
            let lead = matches!(
                w[0],
                Block::Paragraph { .. } | Block::List { .. } | Block::Task { .. }
            );
            let listy = matches!(w[1], Block::List { .. } | Block::Task { .. });
            !(lead && listy)
        })
    });
    let rendered: Vec<String> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let marker = if ordered {
                format!("{}{} ", i + 1, if alt { ')' } else { '.' })
            } else if alt {
                "* ".to_string()
            } else {
                "- ".to_string()
            };
            // A task item: the check box follows the list marker, the task text is the
            // item's first line and the rest of the item hangs below it.
            if let Some(Block::Task { state, inlines, .. }) = item.first() {
                let first = format!("{marker}[{}] ", task_mark(*state));
                let head = paragraph(inlines);
                let rest = blocks(&item[1..], ctx, depth + 1, level, !loose);
                let tight_next =
                    !loose && matches!(item.get(1), Some(Block::List { .. } | Block::Task { .. }));
                let body = match (head.is_empty(), rest.is_empty()) {
                    (_, true) => head,
                    (true, false) => format!("\n{rest}"),
                    (false, false) if tight_next => format!("{head}\n{rest}"),
                    (false, false) => format!("{head}\n\n{rest}"),
                };
                return hang(&first, &body, marker.len());
            }
            let body = blocks(item, ctx, depth + 1, level, !loose);
            hang(&marker, &body, marker.len())
        })
        .collect();
    rendered.join(if loose { "\n\n" } else { "\n" })
}

fn task_mark(state: TaskState) -> char {
    match state {
        TaskState::Open => ' ',
        TaskState::Done => 'x',
        TaskState::Cancelled => '~',
    }
}

/// A task that stands on its own, not inside a list item: a bullet with a check box.
fn task(state: TaskState, content: &[Inline], alt: bool) -> String {
    let mark = task_mark(state);
    let bullet = if alt { '*' } else { '-' };
    let first = format!("{bullet} [{mark}] ");
    hang(&first, &paragraph(content), 2)
}

fn footnote(label: &str, inner: &[Block], ctx: &Ctx, depth: usize) -> String {
    let body = blocks(inner, ctx, depth + 1, 6, false);
    hang(
        &format!("[^{}]: ", footnote_label(label, Mode::Block)),
        &body,
        4,
    )
}

/// The longest info string a fence writes.
const MAX_INFO: usize = 64;

/// A fenced block with a fence longer than any backtick run inside.
pub(crate) fn fence(lang: Option<&str>, text: &str) -> String {
    let fence = "`".repeat(longest_run(text, '`').max(2) + 1);
    // The info string is a language name: its first word, at most 64 characters.
    let info: String = lang
        .unwrap_or("")
        .chars()
        .take_while(|c| !c.is_whitespace())
        .filter(|c| *c != '`')
        .take(MAX_INFO)
        .collect();
    let body = text.strip_suffix('\n').unwrap_or(text);
    if body.is_empty() {
        format!("{fence}{info}\n{fence}")
    } else {
        format!("{fence}{info}\n{body}\n{fence}")
    }
}

fn math_block(latex: &str) -> String {
    let body = latex.trim_matches('\n');
    if body.trim().is_empty() {
        return String::new();
    }
    if math_is_plain(body) {
        format!("$$\n{body}\n$$")
    } else {
        fence(Some("math"), body)
    }
}

/// True when `$$ ... $$` is safe: no blank line, no `$$`, no `<` or backtick, and no line
/// that a plain Markdown reader would take as a block marker.
fn math_is_plain(body: &str) -> bool {
    if body.contains("$$") || body.contains('<') || body.contains('`') {
        return false;
    }
    body.split('\n').all(|line| {
        let t = line.trim_start();
        let Some(c) = t.chars().next() else {
            return false;
        };
        if c.is_ascii_digit() {
            let digits = t.chars().take_while(char::is_ascii_digit).count();
            return !matches!(t[digits..].chars().next(), Some('.' | ')'));
        }
        c.is_alphanumeric()
            || matches!(
                c,
                '\\' | '{'
                    | '}'
                    | '('
                    | ')'
                    | '^'
                    | '_'
                    | '&'
                    | ','
                    | '.'
                    | ';'
                    | ':'
                    | '\''
                    | '!'
                    | '?'
                    | '%'
                    | '/'
                    | '"'
            )
    })
}

/// A picture by its asset `id`; `place` is what this place of it says, when it says more than
/// the asset's alt text (one picture drawn at several places, each with its own box).
fn media(id: &str, place: Option<&str>, ctx: &Ctx) -> String {
    let mut parts: Vec<Inline> = Vec::new();
    let text = Inline::plain;
    let found = ctx.assets.iter().find(|a| a.id == id);
    match found {
        None => parts.push(text(format!("Picture {id} is missing from the assets."))),
        Some(asset) => {
            let alt = place.or(asset.alt.as_deref());
            match &asset.path {
                Some(path) => parts.push(Inline::Image {
                    target: format!("{}{}", ctx.link_base, asset_target(path)),
                    alt: alt
                        .map(str::to_string)
                        .or_else(|| asset.caption.clone())
                        .unwrap_or_else(|| asset.id.clone()),
                    title: None,
                }),
                // The alt text says what the picture is or why it needs a look.
                None => parts.push(text(match alt.map(str::trim) {
                    Some(alt) if !alt.is_empty() => format!(
                        "Picture {} ({}, {alt}) is not extracted.",
                        asset.id, asset.mime
                    ),
                    _ => format!("Picture {} ({}) is not extracted.", asset.id, asset.mime),
                })),
            }
            if let Some(caption) = asset.caption.as_ref().filter(|c| !c.trim().is_empty()) {
                parts.push(text(" ".into()));
                parts.push(Inline::Text {
                    text: caption.clone(),
                    emphasis: true,
                    strong: false,
                    strike: false,
                });
            }
            if asset.needs_vision {
                parts.push(text(" ".into()));
                parts.push(Inline::Status {
                    text: "Needs vision".into(),
                    color: StatusColor::Yellow,
                });
            }
        }
    }
    let mut out = paragraph(&parts);
    let first = found.is_some_and(|a| !a.codes.is_empty())
        && ctx.codes_shown.borrow_mut().insert(id.to_string());
    let codes = if first {
        found.map_or(&[][..], |a| a.codes.as_slice())
    } else {
        &[][..]
    };
    for c in codes {
        out.push_str("\n\n");
        out.push_str(&code_lines(c));
    }
    out
}

/// One code read from a picture: `QR Code: <payload>`. Its payload is data: a web address
/// becomes a link, other text stays escaped text (a payload of several lines a fenced block,
/// control characters as their symbols), bytes are written in base64.
fn code_lines(c: &Barcode) -> String {
    let mut head = c.symbology.clone();
    if let Some(b) = c.bounds {
        let r = |v: f32| v.round() as i64;
        head.push_str(&format!(
            ", box {}, {} to {}, {}",
            r(b[0]),
            r(b[1]),
            r(b[2]),
            r(b[3])
        ));
    }
    head.push_str(": ");
    let mut parts = vec![Inline::plain(head.clone())];
    match (&c.text, &c.base64) {
        (Some(t), _) if t.contains(['\n', '\r']) => {
            return format!(
                "{}\n\n{}",
                paragraph(&[Inline::plain(head.trim_end().to_string())]),
                fence(None, &visible_controls(t))
            );
        }
        (Some(t), _) if web_address(t) => parts.push(Inline::Link {
            target: t.clone(),
            title: None,
            internal: false,
            wiki: false,
            inlines: vec![Inline::plain(t.clone())],
        }),
        (Some(t), _) => parts.push(Inline::plain(visible_controls(t))),
        (None, Some(b64)) => {
            parts.push(Inline::plain(format!("{} bytes, base64 ", c.bytes)));
            parts.push(Inline::Code { text: b64.clone() });
        }
        (None, None) => parts.push(Inline::plain(format!(
            "{} bytes, not kept: more than the reader keeps",
            c.bytes
        ))),
    }
    paragraph(&parts)
}

/// Whether the payload `t` is an address with one of `LINK_SCHEMES` (in the links module) (the rule for links
/// from a document), with something after the scheme and no spaces, control or format
/// characters: only those become links.
fn web_address(t: &str) -> bool {
    let Some((scheme, rest)) = t.split_once(':') else {
        return false;
    };
    let web = scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https");
    let after = if web {
        rest.strip_prefix("//")
    } else {
        Some(rest)
    };
    link_scheme(scheme)
        && after.is_some_and(|a| !a.is_empty())
        && safe_target(t)
        && !t.chars().any(|c| {
            c.is_whitespace() || c.is_control() || format_char(c) || matches!(c, '<' | '>')
        })
}

/// Whether `c` is a Unicode format character (category Cf): invisible, and some turn the
/// text around (U+202E) or hide in a word (U+200B).
fn format_char(c: char) -> bool {
    matches!(
        u32::from(c),
        0xad | 0x600..=0x605
            | 0x61c
            | 0x6dd
            | 0x70f
            | 0x890..=0x891
            | 0x8e2
            | 0x180e
            | 0x200b..=0x200f
            | 0x202a..=0x202e
            | 0x2060..=0x2064
            | 0x2066..=0x206f
            | 0xfeff
            | 0xfff9..=0xfffb
            | 0x110bd
            | 0x110cd
            | 0x13430..=0x1343f
            | 0x1bca0..=0x1bca3
            | 0x1d173..=0x1d17a
            | 0xe0001
            | 0xe0020..=0xe007f
    )
}

/// `t` with its control characters other than tab and line breaks as their Unicode symbols
/// (U+2400 on), so a separator such as GS shows, and format characters as `<U+XXXX>`.
fn visible_controls(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    for c in t.chars() {
        match c {
            '\t' | '\n' | '\r' => out.push(c),
            '\u{0}'..='\u{1f}' => out.push(char::from_u32(0x2400 + c as u32).unwrap_or(c)),
            '\u{7f}' => out.push('\u{2421}'),
            c if format_char(c) => out.push_str(&format!("<U+{:04X}>", u32::from(c))),
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------------------
// Tables

/// The text `[[target]]` or `[[target|text]]` for a link that came from a wikilink, with
/// every bracket, `|` and other Markdown character escaped and line breaks as spaces (ruling
/// P5-AD): Rezzer cannot know the name the target is written under,
/// so a live link to it would be broken on every import. This holds for every wikilink, also
/// one with `_`, `:`, a path or formatting. Crystalline reads a wikilink wherever `[[`
/// stands, also after a backslash, so each bracket is escaped. The link itself stays in the
/// link facts and the graph.
fn wikilink(target: &str, content: &[Inline], mode: Mode, dollars: bool) -> String {
    let text = plain_text(content);
    let spelled = if text.trim().is_empty() || text == target {
        format!("[[{target}]]")
    } else {
        format!("[[{target}|{text}]]")
    };
    let mut out = String::new();
    // With `dollars` every `$` is escaped, as in the text around it.
    escape_with(&spelled, line_mode(mode), dollars, &mut out);
    out
}

#[cfg(test)]
mod tests;
