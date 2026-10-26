// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The object layer in the model: the outline as a section before the first page, links as a
//! graph, attachments, article threads and revisions, and every issue pdfparts found as a
//! loss. The properties and file facts are in `meta.rs`.
//!
//! Every copy put in the model is charged to what the pages left of the bytes the file may
//! keep ([`LinkRoom`]): a fixed cost per item plus the most bytes its texts can be written as.
//! When that or the element budget runs out, what was read stays and one Limit loss names the
//! rest. Links, Launch and remote targets are listed, never followed; attachments are
//! extracted as bytes only when asked, never opened.

use std::cell::Cell as Counter;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub(crate) use pdfparts::objects::quote;
use pdfparts::{Issue, IssueKind, Link, LinkTarget, PageInfo, Pdf, PdfDate, Revision};
use rezzer_limits::Budget;
use rezzer_model::{
    Anchor, Asset, Blob, Block, Edge, Embedded, Event, Graph, Inline, LossReason, Node,
    OutlineEntry, ReadOptions, SectionKind, Severity, link_target_written_len,
};
use sha2::{Digest, Sha256};

use crate::layout::LinkRoom;
use crate::{Out, forms, marker, meta};

/// What one item in the model costs besides its texts: the struct, its strings' headers and
/// the slot in its list.
const ITEM_COST: u64 = 128;

/// Outline levels shown as nested lists. Deeper items stay at the deepest level, so the
/// rendered lists and the cached model stay within their nesting limits.
const OUTLINE_DEPTH: usize = 16;

/// Changes of one revision named in its event; the rest are counted.
const CHANGES_SHOWN: usize = 20;

/// The time limit is checked once per this many items.
const TIME_EVERY: u32 = 1024;

/// Reads the object layer into `out`, after the pages: the page text has the first claim on
/// the bytes the file may keep. `links` are the file's links, read once for the pages too;
/// `events` are the revisions' events, read by [`revisions`] before any list.
#[allow(clippy::too_many_arguments)]
pub(crate) fn read(
    pdf: &Pdf,
    pages: &[PageInfo],
    links: &[Link],
    mut history: History,
    opts: &ReadOptions,
    budget: &Budget,
    room: &LinkRoom,
    out: &mut Out,
) {
    let kept = Kept::new(room, budget, "the metadata, outline and links");
    meta::facts(pdf, out);
    meta::properties(pdf, &kept, out);
    outline(pdf, pages, &kept, out);
    link_graph(links, pages, &kept, out);
    attachments(pdf, opts, &kept, out);
    threads(pdf, &kept, out);
    timeline(std::mem::take(&mut history.events), &kept, out);
    // The form logic has a share of its own, so a large form never costs the outline or
    // links, nor they the form. Its pdfparts issues are among those listed next.
    forms::read(
        pdf,
        links,
        &history,
        &Kept::new(room, budget, "the form logic"),
        out,
    );
    issues(pdf, out);
}

/// The share of the memory limit the object layer's copies may take at most, besides what the
/// pages leave: the renderer and the JSON writer copy what is kept about four times over, and
/// the object layer of a hostile file already takes much of the memory by itself.
const OWN_SHARE: u64 = 64;

/// What one item with copies of `texts` costs: [`ITEM_COST`] plus the most bytes each text
/// can be written as.
fn item_bytes(texts: &[&str]) -> u64 {
    texts
        .iter()
        .map(|t| link_target_written_len(t))
        .fold(ITEM_COST, u64::saturating_add)
}

/// The charges for copies in the model.
pub(crate) struct Kept<'a> {
    room: &'a LinkRoom,
    budget: &'a Budget,
    /// What is left of this share ([`OWN_SHARE`]).
    own: Counter<u64>,
    calls: Counter<u32>,
    /// What the share is for, in the words of its losses.
    what: &'static str,
}

impl<'a> Kept<'a> {
    /// A share of [`OWN_SHARE`] of the memory limit for `what`, beside what the pages leave.
    pub(crate) fn new(room: &'a LinkRoom, budget: &'a Budget, what: &'static str) -> Kept<'a> {
        Kept {
            room,
            budget,
            own: Counter::new(budget.limits().max_memory_bytes / OWN_SHARE),
            calls: Counter::new(0),
            what,
        }
    }

    /// The budget of the read.
    pub(crate) fn budget(&self) -> &Budget {
        self.budget
    }

    /// The share of the memory limit the object layer may keep at most.
    pub(crate) fn share(&self) -> u64 {
        self.budget.limits().max_memory_bytes / OWN_SHARE
    }

    /// As [`Kept::take`], within `cap` too, which it takes from: the room of the keywords and
    /// authors, which must leave room for the others.
    pub(crate) fn take_within(
        &self,
        cap: &Counter<u64>,
        elements: u64,
        texts: &[&str],
    ) -> Result<(), String> {
        let bytes = item_bytes(texts);
        if bytes > cap.get() {
            return Err(format!(
                "the keywords and authors take more than the {} bytes left of their room, half \
                 of what the metadata, outline and links may keep (a 64th of the memory limit)",
                cap.get()
            ));
        }
        self.take(elements, texts)?;
        cap.set(cap.get() - bytes);
        Ok(())
    }

    /// Charges one item of `elements` model elements with copies of `texts`. `Err` says why it
    /// does not fit: then nothing more of its kind may be kept. The bytes are checked first,
    /// so an item that does not fit costs nothing.
    pub(crate) fn take(&self, elements: u64, texts: &[&str]) -> Result<(), String> {
        self.take_with(elements, texts, 0)
    }

    /// As [`Kept::take`], with `extra` bytes the item copies besides its texts.
    pub(crate) fn take_with(
        &self,
        elements: u64,
        texts: &[&str],
        extra: u64,
    ) -> Result<(), String> {
        self.calls.set(self.calls.get().wrapping_add(1));
        if self.calls.get().is_multiple_of(TIME_EVERY)
            && let Err(hit) = self.budget.check_time()
        {
            return Err(hit.to_string());
        }
        let bytes = item_bytes(texts).saturating_add(extra);
        if bytes > self.room.left() {
            return Err(format!(
                "the copies take more than the {} bytes left of what the file may keep",
                self.room.left()
            ));
        }
        let own = self.own.get();
        if bytes > own {
            return Err(format!(
                "the copies take more than the {own} bytes left of what {} may keep (a 64th of \
                 the memory limit)",
                self.what
            ));
        }
        if let Err(hit) = self.budget.add_elements(elements) {
            return Err(hit.to_string());
        }
        let taken = self.room.take_kept(bytes);
        debug_assert!(taken, "checked above");
        self.own.set(own - bytes);
        Ok(())
    }
}

/// `n` things with their verb: "1 keyword is", "3 keywords are".
pub(crate) fn count_is(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one} is")
    } else {
        format!("{n} {many} are")
    }
}

/// "is" or "are" after `n` things.
pub(crate) fn is_are(n: usize) -> &'static str {
    if n == 1 { "is" } else { "are" }
}

/// A text value of the Info dictionary or XMP, trimmed; `None` when empty.
pub(crate) fn given(v: Option<&str>) -> Option<&str> {
    v.map(str::trim).filter(|v| !v.is_empty())
}

/// The ISO form of a date, if it parses.
pub(crate) fn iso(d: Option<&PdfDate>) -> Option<&str> {
    d.and_then(|d| d.iso.as_deref())
}

/// Page `n` (from 1) of `pages`, if it was listed.
fn page_info(pages: &[PageInfo], n: u32) -> Option<&PageInfo> {
    (n as usize)
        .checked_sub(1)
        .and_then(|i| pages.get(i))
        .filter(|p| p.number == n)
}

/// What names a page in the outline: its label, else its number. A label that is a number
/// (`2`) names the same as that number, so page 13 labelled `2` and page 2 without a label
/// share a name.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum PageName<'a> {
    Number(u32),
    Label(&'a str),
}

fn page_name(page: &PageInfo) -> PageName<'_> {
    match page.label.as_deref() {
        Some(l) if !l.is_empty() => match l.parse::<u32>() {
            Ok(v) if v.to_string() == l => PageName::Number(v),
            _ => PageName::Label(l),
        },
        _ => PageName::Number(page.number),
    }
}

/// How the outline names pages: by label or number, and with the page number of the file
/// after a name that more than one page has.
struct PageNames<'a> {
    pages: &'a [PageInfo],
    count: HashMap<PageName<'a>, u32>,
}

impl<'a> PageNames<'a> {
    fn new(pages: &'a [PageInfo]) -> PageNames<'a> {
        let mut count: HashMap<PageName<'a>, u32> = HashMap::new();
        for p in pages {
            *count.entry(page_name(p)).or_default() += 1;
        }
        PageNames { pages, count }
    }

    /// `page iv`, or `page 2 (page 13 of the file)` when page 2 is not the only one called 2.
    fn of(&self, n: u32) -> String {
        let name = page_info(self.pages, n).map_or(PageName::Number(n), page_name);
        let text = match name {
            PageName::Number(n) => n.to_string(),
            PageName::Label(l) => l.to_string(),
        };
        if self.count.get(&name).copied().unwrap_or(0) > 1 {
            format!("page {text} (page {n} of the file)")
        } else {
            format!("page {text}")
        }
    }
}

/// A title on one line: runs of white space and control characters become one space.
fn one_line(text: &str) -> String {
    text.split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Nested lists built from items with levels. A deeper item opens a list in the item before
/// it; a shallower one closes lists down to its level.
struct Nest {
    /// The open lists, outermost first, each with the level of its items.
    open: Vec<(u8, Vec<Vec<Block>>)>,
    /// Items placed at [`OUTLINE_DEPTH`] because they lie deeper.
    flattened: usize,
}

impl Nest {
    fn new() -> Nest {
        Nest {
            open: vec![(0, Vec::new())],
            flattened: 0,
        }
    }

    /// Closes the innermost list into the last item of the one around it.
    fn close(&mut self) {
        let (_, items) = self.open.pop().expect("an inner list");
        let outer = self.open.last_mut().expect("an outer list");
        if let Some(last) = outer.1.last_mut() {
            last.push(Block::List {
                ordered: false,
                items,
                anchor: None,
            });
        }
    }

    /// Places `item` at `level` and returns the depth it is shown at, 1 for the top list.
    fn push(&mut self, level: u8, item: Block) -> u8 {
        while self.open.len() > 1 && self.open.last().is_some_and(|(l, _)| *l > level) {
            self.close();
        }
        let (top_level, top) = self.open.last_mut().expect("a list");
        if top.is_empty() {
            *top_level = level;
        } else if *top_level < level {
            if self.open.len() < OUTLINE_DEPTH {
                self.open.push((level, Vec::new()));
            } else {
                self.flattened += 1;
            }
        }
        let (_, top) = self.open.last_mut().expect("a list");
        top.push(vec![item]);
        // At most OUTLINE_DEPTH lists are open.
        self.open.len() as u8
    }

    fn finish(mut self) -> (Vec<Vec<Block>>, usize) {
        while self.open.len() > 1 {
            self.close();
        }
        let (_, items) = self.open.pop().expect("the outer list");
        (items, self.flattened)
    }
}

/// The outline as a section before the first page: a nested list whose items read
/// `<title>, page <label>` as plain text. An item with a URI action names it in a code span;
/// it is never followed. Each item kept is also an [`OutlineEntry`] in the facets, at the
/// depth the section shows it, with the page of the file it points at.
fn outline(pdf: &Pdf, pages: &[PageInfo], kept: &Kept<'_>, out: &mut Out) {
    let items = pdf.outline();
    if items.is_empty() {
        return;
    }
    let total = items.len();
    let names = PageNames::new(pages);
    let mut nest = Nest::new();
    let mut entries: Vec<OutlineEntry> = Vec::new();
    let mut cut: Option<(usize, String)> = None;
    for (k, item) in items.iter().enumerate() {
        let title = one_line(&item.title);
        let inlines = match (item.page, &item.uri) {
            (Some(p), _) => vec![Inline::plain(format!("{title}, {}", names.of(p)))],
            (None, Some(uri)) => vec![
                Inline::plain(format!("{title}, link to ")),
                Inline::Code {
                    text: uri.to_string(),
                },
            ],
            (None, None) => vec![Inline::plain(title.clone())],
        };
        let texts: Vec<&str> = inlines
            .iter()
            .filter_map(|i| match i {
                Inline::Text { text, .. } | Inline::Code { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        // The item, its paragraph and its inlines, and the entry. The entry's title is a
        // copy of the paragraph's text and shares its charge (P5-L), so the outline keeps as
        // many items as the section alone would.
        if let Err(why) = kept.take(3 + inlines.len() as u64, &texts) {
            cut = Some((k + 1, why));
            break;
        }
        let level = nest.push(item.level, Block::paragraph(inlines));
        entries.push(OutlineEntry {
            level,
            title,
            page: item.page,
        });
    }
    out.doc.facets.outline = entries;
    let (list, flattened) = nest.finish();
    let blocks = if list.is_empty() {
        Vec::new()
    } else {
        vec![Block::List {
            ordered: false,
            items: list,
            anchor: None,
        }]
    };
    out.doc.body.insert(
        0,
        Block::Section {
            kind: SectionKind::Other,
            title: Some("Outline".into()),
            anchor: Some(Anchor("outline".into())),
            blocks,
        },
    );
    if let Some((first, why)) = cut {
        let what = if first == total {
            format!("outline item {first} is")
        } else {
            format!("outline items {first} to {total} are")
        };
        out.loss(
            "outline",
            format!("{what} not kept: {why}"),
            LossReason::Limit,
            Severity::Warning,
        );
    }
    if flattened > 0 {
        out.loss(
            "outline",
            format!(
                "{flattened} outline {} deeper than {OUTLINE_DEPTH} levels {} shown at level \
                 {OUTLINE_DEPTH}",
                if flattened == 1 { "item" } else { "items" },
                is_are(flattened)
            ),
            LossReason::Unsupported,
            Severity::Info,
        );
    }
}

/// The links graph and the link targets that leave the file.
struct LinkGraph {
    graph: Graph,
    pages: HashSet<u32>,
    uris: HashMap<String, String>,
}

impl LinkGraph {
    /// The node of page `n`, added on first use.
    fn page(&mut self, n: u32, pages: &[PageInfo], kept: &Kept<'_>) -> Result<String, String> {
        let id = format!("p{n}");
        if !self.pages.contains(&n) {
            let label = match page_info(pages, n) {
                Some(info) => marker(info),
                None => format!("Page {n}"),
            };
            kept.take(1, &[&id, &label, &id])?;
            self.pages.insert(n);
            self.graph.nodes.push(Node {
                id: id.clone(),
                label,
                kind: Some("page".into()),
                anchor: Some(Anchor(id.clone())),
            });
        }
        Ok(id)
    }

    /// The node of a URI, added on first use.
    fn uri(&mut self, uri: &str, kept: &Kept<'_>) -> Result<String, String> {
        if let Some(id) = self.uris.get(uri) {
            return Ok(id.clone());
        }
        let id = format!("u{}", self.uris.len() + 1);
        // The node's label and the map's key are copies of the URI.
        kept.take(1, &[&id, uri, uri])?;
        self.uris.insert(uri.to_string(), id.clone());
        self.graph.nodes.push(Node {
            id: id.clone(),
            label: uri.to_string(),
            kind: Some("uri".into()),
            anchor: None,
        });
        Ok(id)
    }

    fn edge(&mut self, from: String, to: String, kept: &Kept<'_>) -> Result<(), String> {
        kept.take(1, &[&from, &to])?;
        self.graph.edges.push(Edge {
            from,
            to,
            kind: "links_to".into(),
            anchor: None,
        });
        Ok(())
    }
}

/// Every link annotation: links to pages of this file and to URIs become edges of the graph
/// `links` (one per link, from the page it stands on); Launch, remote and other actions are
/// listed as embedded references. None is followed.
fn link_graph(links: &[Link], pages: &[PageInfo], kept: &Kept<'_>, out: &mut Out) {
    if links.is_empty() {
        return;
    }
    let mut g = LinkGraph {
        graph: Graph {
            id: "links".into(),
            kind: "link".into(),
            nodes: Vec::new(),
            edges: Vec::new(),
        },
        pages: HashSet::new(),
        uris: HashMap::new(),
    };
    let mut dangling: Option<(usize, u32, Arc<str>)> = None;
    let mut cut: Option<(usize, u32, String)> = None;
    for (k, link) in links.iter().enumerate() {
        let from = link.page;
        // An entry is copied only once it is charged, with its target in the same charge.
        let mut listed = |kind: &str, name: &str, detail: String, target: Option<&str>| {
            kept.take(1, &[kind, name, &detail, target.unwrap_or_default()])?;
            out.doc.facets.embedded.push(Embedded {
                kind: kind.into(),
                name: name.to_string(),
                detail: Some(detail),
                target: target.map(str::to_string),
                text: None,
            });
            Ok::<(), String>(())
        };
        let done = match &link.target {
            LinkTarget::Page(to) | LinkTarget::Named(_, Some(to)) => g
                .page(from, pages, kept)
                .and_then(|a| Ok((a, g.page(*to, pages, kept)?)))
                .and_then(|(a, b)| g.edge(a, b, kept)),
            LinkTarget::Named(name, None) => {
                match &mut dangling {
                    Some((n, _, _)) => *n += 1,
                    None => dangling = Some((1, from, name.clone())),
                }
                Ok(())
            }
            LinkTarget::Uri(uri) => g
                .page(from, pages, kept)
                .and_then(|a| Ok((a, g.uri(uri, kept)?)))
                .and_then(|(a, b)| g.edge(a, b, kept)),
            LinkTarget::Remote { file, page } => {
                let detail = match page {
                    Some(p) => format!("from page {from}, to page {p} of that file"),
                    None => format!("from page {from}"),
                };
                listed("remote-link", file, detail, Some(file))
            }
            LinkTarget::Launch(file) => listed(
                "launch-action",
                file,
                format!("from page {from}"),
                Some(file),
            ),
            // A script link is listed with the other scripts, with its text.
            LinkTarget::Other(action) if &**action == "JavaScript" => Ok(()),
            LinkTarget::Other(action) => {
                listed("link-action", action, format!("from page {from}"), None)
            }
        };
        if let Err(why) = done {
            cut = Some((links.len() - k, from, why));
            break;
        }
    }
    if !g.graph.edges.is_empty() {
        out.doc.facets.graphs.push(g.graph);
    }
    if let Some((n, page, name)) = dangling {
        let what = if n == 1 {
            format!(
                "1 internal link names a destination the file does not have ({}); it is not in \
                 the link graph",
                quote(&name)
            )
        } else {
            format!(
                "{n} internal links name a destination the file does not have (the first is {}); \
                 they are not in the link graph",
                quote(&name)
            )
        };
        out.loss(
            format!("p{page}"),
            what,
            LossReason::Damaged,
            Severity::Warning,
        );
    }
    if let Some((n, page, why)) = cut {
        out.loss(
            format!("p{page}"),
            format!(
                "{n} more link {} not kept, from the one on page {page} on: {why}",
                if n == 1 { "entry is" } else { "entries are" }
            ),
            LossReason::Limit,
            Severity::Warning,
        );
    }
}

/// The attachments, listed with type, size and description. With `extract_assets` each one's
/// bytes come back as a blob, within what is left of the eighth of the memory limit that
/// rendered pages and attachments share; the ones left out get one Limit loss together.
fn attachments(pdf: &Pdf, opts: &ReadOptions, kept: &Kept<'_>, out: &mut Out) {
    let files = pdf.embedded_files();
    let mut room = out.blob_room;
    // The attachments not extracted for a limit: the first one, why, and how many.
    let mut left_out: Option<(usize, String, usize)> = None;
    for (i, f) in files.iter().enumerate() {
        let mime = given(f.mime.as_deref());
        let description = given(f.description.as_deref());
        // The size the guard measured; the one the file states only when there is none.
        let size = match (f.size, pdf.embedded_file_measured(i)) {
            (Some(stated), Some(measured)) if stated != measured => {
                Some(format!("{stated} bytes stated, {measured} unpacked"))
            }
            (_, Some(n)) | (Some(n), None) => Some(format!("{n} bytes")),
            (None, None) => None,
        };
        let size = size.unwrap_or_default();
        let texts = [
            &*f.name,
            mime.unwrap_or_default(),
            &size,
            description.unwrap_or_default(),
        ];
        if let Err(why) = kept.take(1, &texts) {
            let left = files.len() - i;
            out.loss(
                format!("attachment {}", i + 1),
                format!(
                    "{left} of {} attachments {} not listed, from attachment {} on: {why}",
                    files.len(),
                    is_are(left),
                    i + 1
                ),
                LossReason::Limit,
                Severity::Warning,
            );
            break;
        }
        let detail = [
            mime,
            (!size.is_empty()).then_some(size.as_str()),
            description,
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");
        out.doc.facets.embedded.push(Embedded {
            kind: "attachment".into(),
            name: f.name.to_string(),
            detail: (!detail.is_empty()).then_some(detail),
            target: None,
            text: None,
        });
        if opts.extract_assets
            && let Err(why) = extract(pdf, i, &f.name, mime, &mut room, kept, out)
        {
            match &mut left_out {
                Some((_, _, n)) => *n += 1,
                None => left_out = Some((i, why, 1)),
            }
        }
    }
    out.blob_room = room;
    if let Some((i, why, n)) = left_out {
        let name = &files[i].name;
        let what = if n == 1 {
            format!("{} is not extracted: {why}", quote(name))
        } else {
            format!(
                "{n} attachments are not extracted, the first {} (attachment {}): {why}",
                quote(name),
                i + 1
            )
        };
        out.loss(
            format!("attachment {}", i + 1),
            what,
            LossReason::Limit,
            Severity::Warning,
        );
    }
}

/// The bytes of attachment `i` as asset `attachment-<i+1>` and its blob. The length the guard
/// measured is checked before anything is unpacked. `Err` says why a limit left it out; a
/// file pdfparts cannot unpack gets its own loss.
fn extract(
    pdf: &Pdf,
    i: usize,
    name: &str,
    mime: Option<&str>,
    room: &mut u64,
    kept: &Kept<'_>,
    out: &mut Out,
) -> Result<(), String> {
    let over = |n: u64, room: u64| {
        format!(
            "it unpacks to {n} bytes, more than the {room} bytes left for extracted pictures \
             and attachments (an eighth of the memory limit)"
        )
    };
    if let Some(n) = pdf.embedded_file_measured(i)
        && n > *room
    {
        return Err(over(n, *room));
    }
    let bytes = match pdf.embedded_file_bytes(i) {
        Ok(bytes) => bytes,
        Err(issue) => {
            let (why, severity) = reason(issue.kind);
            out.loss(
                format!("attachment {}", i + 1),
                format!("{}: {}", quote(name), issue.what),
                why,
                severity,
            );
            return Ok(());
        }
    };
    let n = bytes.len() as u64;
    if n > *room {
        return Err(over(n, *room));
    }
    let id = format!("attachment-{}", i + 1);
    let mime = mime.unwrap_or("application/octet-stream").to_string();
    let sha256: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    kept.take(1, &[&id, &id, &mime, name, &sha256])?;
    *room -= n;
    out.doc.assets.push(Asset {
        id: id.clone(),
        path: None,
        sha256,
        mime,
        caption: Some(name.to_string()),
        alt: None,
        needs_vision: false,
        codes: Vec::new(),
        code_unread: false,
    });
    out.blobs.push(Blob { asset: id, bytes });
    Ok(())
}

/// "page 1" or "pages 1, 2, 3".
fn page_list(pages: &[u32]) -> String {
    let list: Vec<String> = pages.iter().map(u32::to_string).collect();
    let word = if pages.len() == 1 { "page" } else { "pages" };
    format!("{word} {}", list.join(", "))
}

/// The article threads with their pages.
fn threads(pdf: &Pdf, kept: &Kept<'_>, out: &mut Out) {
    let threads = pdf.threads();
    for (k, t) in threads.iter().enumerate() {
        let fallback = format!("thread {}", k + 1);
        let name = given(t.title.as_deref()).unwrap_or(&fallback);
        let detail = page_list(&t.pages);
        if let Err(why) = kept.take(1, &[name, &detail]) {
            let left = threads.len() - k;
            out.loss(
                "article threads",
                format!(
                    "{left} of {} article threads {} not listed, from thread {} on: {why}",
                    threads.len(),
                    is_are(left),
                    k + 1
                ),
                LossReason::Limit,
                Severity::Warning,
            );
            return;
        }
        out.doc.facets.embedded.push(Embedded {
            kind: "article-thread".into(),
            name: name.to_string(),
            detail: Some(detail),
            target: None,
            text: None,
        });
    }
}

/// The words of one revision's event: `<n> changes: <the first 20 joined with '; '>, and <m>
/// more`.
fn revision_text(rev: &Revision) -> String {
    let n = rev.changes.len();
    if n == 0 {
        return if rev.number == 1 {
            "first save".into()
        } else {
            "no changes".into()
        };
    }
    let shown = rev.changes[..n.min(CHANGES_SHOWN)].join("; ");
    let word = if n == 1 { "change" } else { "changes" };
    let mut text = format!("{n} {word}: {shown}");
    if n > CHANGES_SHOWN {
        text.push_str(&format!(", and {} more", n - CHANGES_SHOWN));
    }
    text
}

/// The revisions' events: number, time and words.
pub(crate) type Events = Vec<(u32, Option<String>, String)>;

/// What the revisions tell the rest of the object layer: the events, and where each save
/// ends, oldest first, for the saves after a signature.
pub(crate) struct History {
    events: Events,
    /// Where each kept save ends, oldest first.
    pub ends: Vec<u64>,
    /// Older saves that are counted but not compared.
    pub skipped: usize,
}

/// The revisions: their count as a fact, their issues as losses, and the time and words of
/// one event per revision, when the file has 2 or more (a file saved once has no history to
/// tell). Older revisions over the cap are counted, not compared: that is one Limit Info loss.
/// They are read before the links and every other list, which draw on the same pdfparts
/// budget: they are few and small.
pub(crate) fn revisions(pdf: &Pdf, budget: &Budget, out: &mut Out) -> History {
    let stop = || budget.check_time().is_err();
    let r = pdf.revisions(&stop);
    let count = r.list.len() + r.skipped;
    if count > 0 {
        out.doc.properties.facts.revisions = Some(u32::try_from(count).unwrap_or(u32::MAX));
    }
    if let Some(issue) = &r.skipped_issue {
        out.loss(
            location(issue),
            issue.what.clone(),
            LossReason::Limit,
            Severity::Info,
        );
    }
    for issue in &r.issues {
        let (why, severity) = reason(issue.kind);
        out.loss(location(issue), issue.what.clone(), why, severity);
    }
    let ends = r.list.iter().map(|rev| rev.end).collect();
    if count < 2 {
        return History {
            events: Vec::new(),
            ends,
            skipped: r.skipped,
        };
    }
    let events = r
        .list
        .iter()
        .map(|rev| {
            let time = iso(rev.modified.as_ref()).map(str::to_string);
            (rev.number, time, revision_text(rev))
        })
        .collect();
    History {
        events,
        ends,
        skipped: r.skipped,
    }
}

/// One timeline event per revision, charged like every other copy.
fn timeline(events: Events, kept: &Kept<'_>, out: &mut Out) {
    let last = events.last().map_or(0, |(n, _, _)| *n);
    for (number, time, text) in events {
        if let Err(why) = kept.take(1, &[&text, time.as_deref().unwrap_or_default()]) {
            out.loss(
                "revisions",
                format!("the events of revisions {number} to {last} are not kept: {why}"),
                LossReason::Limit,
                Severity::Warning,
            );
            break;
        }
        out.doc.facets.timeline.push(Event {
            time,
            actor: None,
            kind: "revision".into(),
            anchor: None,
            text: Some(text),
        });
    }
}

/// The loss reason and severity of an issue kind.
pub(crate) fn reason(kind: IssueKind) -> (LossReason, Severity) {
    match kind {
        IssueKind::Damaged => (LossReason::Damaged, Severity::Warning),
        IssueKind::Limit => (LossReason::Limit, Severity::Warning),
        IssueKind::Unsupported => (LossReason::Unsupported, Severity::Info),
        IssueKind::Uncertain => (LossReason::Uncertain, Severity::Info),
    }
}

/// Where an issue is: `page 3` becomes the page anchor `p3`.
fn location(issue: &Issue) -> String {
    match issue.location.strip_prefix("page ") {
        Some(n) if n.parse::<u32>().is_ok() => format!("p{n}"),
        _ => issue.location.clone(),
    }
}

/// The issues pdfparts found while opening and for every reader called so far, as losses.
fn issues(pdf: &Pdf, out: &mut Out) {
    for issue in pdf.issues() {
        let (why, severity) = reason(issue.kind);
        out.loss(location(&issue), issue.what, why, severity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn para(t: &str) -> Block {
        Block::paragraph(vec![Inline::plain(t)])
    }

    /// The item texts of nested lists with their depth.
    fn walk(items: &[Vec<Block>], depth: usize, out: &mut Vec<(usize, String)>) {
        for item in items {
            for b in item {
                match b {
                    Block::Paragraph { inlines, .. } => {
                        out.push((depth, rezzer_model::plain_text(inlines)));
                    }
                    Block::List { items, .. } => walk(items, depth + 1, out),
                    other => panic!("{other:?}"),
                }
            }
        }
    }

    #[test]
    fn levels_nest_and_jumps_stay_one_deeper() {
        let mut nest = Nest::new();
        for (level, t) in [(1, "a"), (3, "b"), (3, "c"), (2, "d"), (1, "e"), (0, "f")] {
            nest.push(level, para(t));
        }
        let (items, flattened) = nest.finish();
        let mut seen = Vec::new();
        walk(&items, 1, &mut seen);
        let want: Vec<(usize, String)> =
            [(1, "a"), (2, "b"), (2, "c"), (2, "d"), (1, "e"), (1, "f")]
                .iter()
                .map(|(d, t)| (*d, t.to_string()))
                .collect();
        assert_eq!(seen, want);
        assert_eq!(flattened, 0);
    }

    #[test]
    fn deep_outlines_stop_at_the_depth_cap() {
        let mut nest = Nest::new();
        for level in 1..=40u8 {
            nest.push(level, para(&level.to_string()));
        }
        let (items, flattened) = nest.finish();
        let mut seen = Vec::new();
        walk(&items, 1, &mut seen);
        assert_eq!(seen.len(), 40);
        assert_eq!(seen.iter().map(|(d, _)| *d).max(), Some(OUTLINE_DEPTH));
        assert_eq!(flattened, 40 - OUTLINE_DEPTH);
    }

    #[test]
    fn revision_words() {
        let rev = |number: u32, n: usize| Revision {
            number,
            end: 0,
            modified: None,
            changes: (1..=n).map(|k| format!("c{k}")).collect(),
        };
        assert_eq!(revision_text(&rev(1, 0)), "first save");
        assert_eq!(revision_text(&rev(4, 0)), "no changes");
        assert_eq!(revision_text(&rev(2, 1)), "1 change: c1");
        assert_eq!(revision_text(&rev(2, 3)), "3 changes: c1; c2; c3");
        let many = revision_text(&rev(2, 22));
        assert!(many.starts_with("22 changes: c1; c2;"), "{many}");
        assert!(many.ends_with("c20, and 2 more"), "{many}");
    }

    #[test]
    fn titles_on_one_line() {
        assert_eq!(one_line("  A\n\tB\u{0}C  "), "A B C");
    }
}
