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
    /// Which notes the content shows.
    pub level: crate::NoteLevel,
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
        | Block::Spatial { anchor, .. }
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
/// starts with a block marker gets it escaped. A no-break space at a line start is no indent
/// to Markdown and stays: a line kept as laid out is indented with it.
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
        // A no-break space is the indent a line kept as laid out is written with.
        let blank = |c: char| c.is_whitespace() && c != '\u{a0}';
        let line = if first {
            line.trim_start_matches(blank)
        } else {
            line
        };
        first = false;
        let indent = line.len() - line.trim_start_matches(blank).len();
        out.push_str(&line[..indent]);
        escape_line_start(&line[indent..], &mut out);
    }
    out
}

/// Writes `line`, a block marker at its start escaped where it would open a block: `#`, `>`
/// and `=` always (a heading or a tag, a quote, the underline of a heading); `-` and `+`
/// before a space or the line end (a list item) and a line of nothing but `-`, spaces and
/// tabs (the underline of a heading, or a rule: three hyphens or more are a thematic break
/// with blanks between them too, `-- -` and `-----   -----`); digits with `.` or `)` before
/// a space or the line end (a numbered item). `-414`, `-- x`, `03.04.2026` and `3.1 Scope`
/// open nothing and stay as they are. A rule of `*` or `_` needs no rule here: those are
/// escaped wherever they stand outside a word.
pub(crate) fn escape_line_start(line: &str, out: &mut String) {
    // A list marker opens an item only before white space or at the line end.
    let opens = |rest: &str| rest.chars().next().is_none_or(|c| c == ' ' || c == '\t');
    let escaped = match line.chars().next() {
        Some('#' | '>' | '=') => Some(0),
        Some('-') => {
            (opens(&line[1..]) || line.chars().all(|c| matches!(c, '-' | ' ' | '\t'))).then_some(0)
        }
        Some('+') => opens(&line[1..]).then_some(0),
        Some(c) if c.is_ascii_digit() => {
            let digits = line.chars().take_while(char::is_ascii_digit).count();
            let marker = line[digits..].starts_with(['.', ')']);
            (marker && opens(&line[digits + 1..])).then_some(digits)
        }
        _ => None,
    };
    match escaped {
        Some(at) => {
            out.push_str(&line[..at]);
            out.push('\\');
            out.push_str(&line[at..]);
        }
        None => out.push_str(line),
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
    // Blocks kept as laid out that stand in the same columns are written once, as one
    // aligned block where the first of them stands.
    let runs = rezzer_model::spatial_runs(list);
    let mut in_run: HashMap<usize, Option<&[usize]>> = HashMap::new();
    for run in &runs {
        for (i, &k) in run.iter().enumerate() {
            in_run.insert(k, (i == 0).then_some(run.as_slice()));
        }
    }
    for (k, item) in list.iter().enumerate() {
        let (text, now) = match item {
            Block::Spatial { .. } if in_run.contains_key(&k) => {
                let Some(Some(run)) = in_run.get(&k) else {
                    continue;
                };
                let mut text = spatial_run(list, run);
                for &member in *run {
                    text = with_notes(text, passage_anchor(&list[member]), ctx, depth, level);
                }
                (
                    text,
                    Prev {
                        kind: PrevKind::Other,
                        alt: false,
                        task: false,
                        tight_lead: false,
                    },
                )
            }
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
            inlines: content,
            role: Some(role),
            ..
        } if role == rezzer_model::FOUND_BY_ROLE
            && !crate::levels::found_by_shown(&plain_text(content), ctx.level) =>
        {
            String::new()
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
        } => {
            let rows = with_cell_media(rows, ctx);
            match layout {
                Some(layout) => crate::layout::layout_table(
                    layout,
                    *header_rows,
                    &rows,
                    alignments,
                    true,
                    ctx,
                    depth,
                    level,
                ),
                None => model_table(
                    caption.as_deref(),
                    *header_rows,
                    &rows,
                    alignments,
                    ctx.level,
                ),
            }
        }
        Block::Code { lang, text, .. } => fence(lang.as_deref(), text),
        Block::Spatial { lines, links, .. } => spatial_block(lines, links),
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

/// The widest gap a fence of blocks kept as laid out pads before a piece, in characters. A
/// piece further from the end of the one before it (or from the left edge of the fence) is
/// written after one space, like an overlapping piece: a column of the file bounds no line.
const MAX_SPATIAL_PAD: u64 = 1024;

/// A run of blocks kept as laid out in the same columns ([`rezzer_model::spatial_runs`];
/// `run` holds their indices in `list`): one `text` fence over the lines of all of them, each
/// piece at its character column, counted from the smallest column of the run. The reader
/// that joined the blocks counted the columns of all of them on one grid of the page
/// ([`rezzer_model::SpatialGrid`]), so a piece is written under what it stands under. Here
/// the place of a piece is meaning (a value under its label), so the aligned form is the
/// honest one. A piece that starts before the end of the one before it, or further than
/// [`MAX_SPATIAL_PAD`], gets one space: no text is lost, only the alignment. A line without
/// text is left out, a line break inside a piece is a space, the text is verbatim and a line
/// has no trailing spaces. A fence shows no link: when the text holds links or footnote
/// references, a paragraph right after the fence says on which lines they stand
/// ([`spatial_key`]).
fn spatial_run(list: &[Block], run: &[usize]) -> String {
    let parts = || {
        run.iter().filter_map(|&k| match &list[k] {
            Block::Spatial { lines, links, .. } => Some((lines.as_slice(), links.as_slice())),
            _ => None,
        })
    };
    let written = |p: &&rezzer_model::SpatialPiece| !p.text.trim().is_empty();
    let left = parts()
        .flat_map(|(lines, _)| lines)
        .flat_map(|l| l.pieces.iter().filter(written))
        .map(|p| u64::from(p.col))
        .min()
        .unwrap_or(0);
    let mut body = String::new();
    // The lines of the fence, each with the links of its block.
    let mut rows: Vec<(&rezzer_model::SpatialLine, &[String])> = Vec::new();
    for (lines, links) in parts() {
        for line in lines {
            if !line.pieces.iter().any(|p| written(&p)) {
                continue;
            }
            if !rows.is_empty() {
                body.push('\n');
            }
            rows.push((line, links));
            // The column after the last piece written, in the fence.
            let mut at = 0u64;
            for (n, piece) in line.pieces.iter().filter(written).enumerate() {
                let col = u64::from(piece.col) - left;
                let pad = if col > at && col - at <= MAX_SPATIAL_PAD {
                    col - at
                } else {
                    u64::from(n > 0)
                };
                body.extend(std::iter::repeat_n(' ', pad as usize));
                let text = piece.text.trim_end();
                body.extend(text.chars().map(|c| {
                    if matches!(
                        c,
                        '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}'
                    ) {
                        ' '
                    } else {
                        c
                    }
                }));
                at += pad + text.chars().count() as u64;
            }
        }
    }
    let fenced = fence(Some("text"), &body);
    let key = spatial_key(&rows);
    if key.is_empty() {
        fenced
    } else {
        format!("{fenced}\n\n{}", paragraph(&key))
    }
}

/// The links and footnote references of a fence of blocks kept as laid out, by line of the
/// fence (counted from 1): `Links: [line 2](target), [lines 4 to 6 and 9](other). Footnote
/// references: line 1[^a], line 3[^b].` Each target is written once, with the lines it
/// stands on; each reference where it stands. A span that points to no link of its block is
/// left out (its text stands in the fence).
fn spatial_key(rows: &[(&rezzer_model::SpatialLine, &[String])]) -> Vec<Inline> {
    // Per target, in the order they first stand, the lines it stands on, each once.
    let mut on: Vec<(&str, Vec<usize>)> = Vec::new();
    let mut refs: Vec<(usize, &str)> = Vec::new();
    for (n, (line, links)) in rows.iter().enumerate() {
        for span in line.pieces.iter().flat_map(|p| &p.spans) {
            if let Some(target) = span
                .link
                .and_then(|k| usize::try_from(k).ok())
                .and_then(|k| links.get(k))
            {
                match on.iter_mut().find(|(t, _)| *t == target.as_str()) {
                    Some((_, at)) if at.last() == Some(&(n + 1)) => {}
                    Some((_, at)) => at.push(n + 1),
                    None => on.push((target.as_str(), vec![n + 1])),
                }
            }
            if let Some(id) = &span.footnote {
                refs.push((n + 1, id));
            }
        }
    }
    let mut out = Vec::new();
    if !on.is_empty() {
        out.push(Inline::plain("Links: "));
        for (i, (target, lines)) in on.iter().enumerate() {
            if i > 0 {
                out.push(Inline::plain(", "));
            }
            out.push(Inline::Link {
                target: (*target).to_string(),
                title: None,
                internal: false,
                wiki: false,
                inlines: vec![Inline::plain(line_numbers(lines))],
            });
        }
        out.push(Inline::plain("."));
    }
    if !refs.is_empty() {
        out.push(Inline::plain(if out.is_empty() {
            "Footnote references: "
        } else {
            " Footnote references: "
        }));
        let mut last = 0;
        for &(n, id) in &refs {
            if n != last {
                if last > 0 {
                    out.push(Inline::plain(", "));
                }
                out.push(Inline::plain(format!("line {n}")));
                last = n;
            }
            out.push(Inline::FootnoteRef { id: id.to_string() });
        }
        out.push(Inline::plain("."));
    }
    out
}

/// `line 3`, `lines 2 and 5`, `lines 4 to 6 and 9`: ascending line numbers, runs of three or
/// more as a range.
fn line_numbers(ns: &[usize]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < ns.len() {
        let mut j = i;
        while j + 1 < ns.len() && ns[j + 1] == ns[j] + 1 {
            j += 1;
        }
        match j - i {
            0 => parts.push(ns[i].to_string()),
            1 => {
                parts.push(ns[i].to_string());
                parts.push(ns[j].to_string());
            }
            _ => parts.push(format!("{} to {}", ns[i], ns[j])),
        }
        i = j + 1;
    }
    let word = if ns.len() == 1 { "line" } else { "lines" };
    match parts.split_last() {
        None => String::new(),
        Some((last, [])) => format!("{word} {last}"),
        Some((last, rest)) => format!("{word} {} and {last}", rest.join(", ")),
    }
}

/// The space between two pieces of a line that stand apart in the layout. Pieces that touch
/// get one space. The gap is wider than a space between words, so a reader of the Markdown
/// sees where one piece ends, and it is the same for every distance: the lines are not
/// aligned in columns.
const PIECE_GAP: &str = "   ";

/// The widest indent the plain lines of a block kept as laid out are written with, in
/// characters. The indent of a line shows that the line is indented and how far against the
/// other lines of its block: eight levels of a listing set four columns a level fit in it as
/// they stand. A block with a wider indent (a line far to the right on a wide page) has all
/// its indents scaled down so that the widest is this one. Written out, such indents can be
/// most of the Markdown, and they are text the limits of a read never counted. The columns
/// themselves stay in the JSON.
const MAX_LINE_INDENT: u64 = 32;

/// Text of a region no type had clear evidence for: its lines top to bottom as one paragraph,
/// each line of the layout a line of its own (a hard break), the pieces of a line left to
/// right. Nothing is aligned: the columns of the pieces stay in the JSON. Only the indent of
/// a line against the leftmost line of the block is written, as no-break spaces (a listing
/// reads by its indent); one further than [`MAX_SPATIAL_PAD`] is left out, and the indents of
/// a block whose widest is over [`MAX_LINE_INDENT`] are scaled down to it, in their order.
/// Bold, italic, links and footnote references are written as in any paragraph, and the
/// text is escaped. A line without text is left out; a line break inside a piece is written
/// as a space (a piece is one line).
fn spatial_block(lines: &[rezzer_model::SpatialLine], links: &[String]) -> String {
    let written = |p: &&rezzer_model::SpatialPiece| !p.text.trim().is_empty();
    // The column the leftmost line starts at: the indent of a line is counted from it.
    let left = lines
        .iter()
        .filter_map(|l| l.pieces.iter().find(written))
        .map(|p| u64::from(p.col))
        .min()
        .unwrap_or(0);
    // The widest indent written: a wider one sets the scale of all.
    let widest = lines
        .iter()
        .filter_map(|l| l.pieces.iter().find(written))
        .map(|p| u64::from(p.col) - left)
        .filter(|i| *i <= MAX_SPATIAL_PAD)
        .max()
        .unwrap_or(0);
    // An indent as written: as it stands, or its share of the widest one (never none for a
    // line that is indented).
    let scaled = |indent: u64| {
        if widest <= MAX_LINE_INDENT {
            indent
        } else {
            (indent * MAX_LINE_INDENT).div_ceil(widest)
        }
    };
    let mut out: Vec<Inline> = Vec::new();
    for line in lines {
        let mut row: Vec<Inline> = Vec::new();
        // The column after the last piece written, as the layout counts it.
        let mut at = 0u64;
        for piece in line.pieces.iter().filter(written) {
            if row.is_empty() {
                // A no-break space is no indent to Markdown (no code block, no list level)
                // and stays at the start of a line of a paragraph.
                let indent = u64::from(piece.col).saturating_sub(left);
                if (1..=MAX_SPATIAL_PAD).contains(&indent) {
                    row.push(Inline::plain("\u{a0}".repeat(scaled(indent) as usize)));
                }
            } else {
                let apart = u64::from(piece.col) > at + 1;
                row.push(Inline::plain(if apart { PIECE_GAP } else { " " }));
            }
            spatial_piece(piece, links, &mut row);
            at = u64::from(piece.col) + piece.text.chars().count() as u64;
        }
        if row.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(Inline::LineBreak);
        }
        out.append(&mut row);
    }
    paragraph(&out)
}

/// The inlines of one piece of a spatial line: its text without the white space around it,
/// each span as what it is. A span that is a footnote reference is written as the reference
/// in place of the marker as printed; spans of one link that follow one another are one link.
/// A span that reaches past the text, starts before the end of the one before it or points
/// to no link of the block is cut or written as plain text: no text is lost.
fn spatial_piece(piece: &rezzer_model::SpatialPiece, links: &[String], out: &mut Vec<Inline>) {
    let chars: Vec<char> = piece
        .text
        .chars()
        .map(|c| {
            if matches!(
                c,
                '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}'
            ) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let text = |from: usize, to: usize| chars[from..to].iter().collect::<String>();
    // The white space around the piece is its place on the line, which the gap before it
    // says: only what stands between is written.
    let blank = |c: &char| c.is_whitespace();
    let last = chars.len() - chars.iter().rev().take_while(|c| blank(c)).count();
    let mut at = chars.iter().take_while(|c| blank(c)).count().min(last);
    // The link the span before this one wrote, and where it ended: a span that goes on with
    // the same link right there (another style in the link's text) joins it.
    let mut linked: Option<(u32, usize)> = None;
    for span in &piece.spans {
        let start = (span.start as usize).clamp(at, last);
        let end = (span.end as usize).clamp(start, last);
        if start > at {
            out.push(Inline::plain(text(at, start)));
            at = start;
        }
        if end == start {
            continue;
        }
        at = end;
        if let Some(id) = &span.footnote {
            out.push(Inline::FootnoteRef { id: id.clone() });
            linked = None;
            continue;
        }
        let inner = Inline::Text {
            text: text(start, end),
            emphasis: span.emphasis,
            strong: span.strong,
            strike: false,
        };
        let link = span
            .link
            .filter(|&k| usize::try_from(k).is_ok_and(|k| k < links.len()));
        let Some(k) = link else {
            out.push(inner);
            linked = None;
            continue;
        };
        match out.last_mut() {
            Some(Inline::Link { inlines, .. }) if linked == Some((k, start)) => inlines.push(inner),
            _ => out.push(Inline::Link {
                target: links[k as usize].clone(),
                title: None,
                internal: false,
                wiki: false,
                inlines: vec![inner],
            }),
        }
        linked = Some((k, end));
    }
    if at < last {
        out.push(Inline::plain(text(at, last)));
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

/// `rows` with the pictures of each cell (see [`rezzer_model::CellMedia`]) written after its
/// text as [`media_inlines`] writes them, a space between; borrowed when no cell has one.
fn with_cell_media<'r>(rows: &'r [Vec<Cell>], ctx: &Ctx) -> Cow<'r, [Vec<Cell>]> {
    if rows.iter().flatten().all(|c| c.media.is_empty()) {
        return Cow::Borrowed(rows);
    }
    let rows = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|c| {
                    let mut c = c.clone();
                    for m in std::mem::take(&mut c.media) {
                        if !c.inlines.is_empty() {
                            c.inlines.push(Inline::plain(" "));
                        }
                        c.inlines
                            .extend(media_inlines(&m.asset, m.alt.as_deref(), ctx));
                        // The codes read from the picture, at its first place, in its cell.
                        let found = ctx.assets.iter().find(|a| a.id == m.asset);
                        let first = found.is_some_and(|a| !a.codes.is_empty())
                            && ctx.codes_shown.borrow_mut().insert(m.asset.clone());
                        if first {
                            for code in found.map_or(&[][..], |a| a.codes.as_slice()) {
                                c.inlines.push(Inline::plain(" "));
                                c.inlines.extend(code_inlines(code));
                            }
                        }
                    }
                    c
                })
                .collect()
        })
        .collect();
    Cow::Owned(rows)
}

/// A picture by its asset `id`; `place` is what this place of it says, when it says more than
/// the asset's alt text (one picture drawn at several places, each with its own box).
fn media(id: &str, place: Option<&str>, ctx: &Ctx) -> String {
    let found = ctx.assets.iter().find(|a| a.id == id);
    let mut out = paragraph(&media_inlines(id, place, ctx));
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

/// The inlines of a picture by its asset `id` (see [`media`]): the picture, or why it is not
/// extracted, its caption, and whether it needs vision.
fn media_inlines(id: &str, place: Option<&str>, ctx: &Ctx) -> Vec<Inline> {
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
    parts
}

/// One code read from a picture as the inlines of one line (for a table cell, see
/// [`with_cell_media`]): its symbology and place, then its payload, a payload of several lines
/// with its line ends as their symbols.
fn code_inlines(c: &Barcode) -> Vec<Inline> {
    let head = match &c.place {
        Some(place) => format!("{}, {place}: ", c.symbology),
        None => format!("{}: ", c.symbology),
    };
    let mut parts = vec![Inline::plain(head)];
    match (&c.text, &c.base64) {
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
    parts
}

/// One code read from a picture: `QR Code: <payload>`, with its place on the page when it
/// has one (`Code 128, top left of the page: <payload>`); its box in points is in the JSON
/// only. Its payload is data: a web address becomes a link, other text stays escaped text (a
/// payload of several lines a fenced block, control characters as their symbols), bytes are
/// written in base64.
fn code_lines(c: &Barcode) -> String {
    let head = match &c.place {
        Some(place) => format!("{}, {place}: ", c.symbology),
        None => format!("{}: ", c.symbology),
    };
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

/// Whether the payload `t` is an address with one of [`LINK_SCHEMES`] (the rule for links from a
/// document), with something after the scheme and no spaces, control or format characters: only
/// those become links.
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
/// every bracket, `|` and other Markdown character escaped and line breaks as spaces:
/// Rezzer cannot know the name the target is written under,
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
