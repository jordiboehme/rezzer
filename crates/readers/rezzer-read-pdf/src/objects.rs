// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The object layer in the model: the Info dictionary and XMP as properties, the file facts,
//! the outline as a section before the first page, links as a graph, attachments, article
//! threads, layers and revisions, and every issue pdfparts found as a loss.
//!
//! Every copy put in the model is charged to what the pages left of the bytes the file may
//! keep ([`LinkRoom`]): a fixed cost per item plus the most bytes its texts can be written as.
//! When that or the element budget runs out, what was read stays and one Limit loss names the
//! rest. Links, Launch and remote targets are listed, never followed; attachments are
//! extracted as bytes only when asked, never opened.

use std::cell::Cell as Counter;
use std::collections::{HashMap, HashSet};

use pdfparts::{Info, Issue, IssueKind, LinkTarget, PageInfo, Pdf, PdfDate, Revision};
use rezzer_limits::Budget;
use rezzer_model::{
    Anchor, Asset, Blob, Block, Edge, Embedded, Event, Graph, Inline, LossReason, Node,
    ReadOptions, SectionKind, Severity, Value, link_target_written_len,
};
use sha2::{Digest, Sha256};

use crate::layout::LinkRoom;
use crate::{Out, marker};

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
/// the bytes the file may keep.
pub(crate) fn read(
    pdf: &Pdf,
    pages: &[PageInfo],
    opts: &ReadOptions,
    budget: &Budget,
    room: &LinkRoom,
    out: &mut Out,
) {
    let kept = Kept {
        room,
        budget,
        calls: Counter::new(0),
    };
    facts(pdf, out);
    properties(pdf, &kept, out);
    outline(pdf, pages, &kept, out);
    links(pdf, pages, &kept, out);
    attachments(pdf, opts, budget, &kept, out);
    threads(pdf, &kept, out);
    revisions(pdf, budget, &kept, out);
    issues(pdf, out);
}

/// The charges for copies in the model.
struct Kept<'a> {
    room: &'a LinkRoom,
    budget: &'a Budget,
    calls: Counter<u32>,
}

impl Kept<'_> {
    /// Charges one item of `elements` model elements with copies of `texts`. `Err` says why it
    /// does not fit: then nothing more of its kind may be kept.
    fn take(&self, elements: u64, texts: &[&str]) -> Result<(), String> {
        self.calls.set(self.calls.get().wrapping_add(1));
        if self.calls.get().is_multiple_of(TIME_EVERY)
            && let Err(hit) = self.budget.check_time()
        {
            return Err(hit.to_string());
        }
        if let Err(hit) = self.budget.add_elements(elements) {
            return Err(hit.to_string());
        }
        let bytes = texts
            .iter()
            .map(|t| link_target_written_len(t))
            .fold(ITEM_COST, u64::saturating_add);
        if self.room.take_kept(bytes) {
            Ok(())
        } else {
            Err(format!(
                "the copies take more than the {} bytes left of what the file may keep",
                self.room.left()
            ))
        }
    }
}

/// "is" or "are" after `n` things.
fn is_are(n: usize) -> &'static str {
    if n == 1 { "is" } else { "are" }
}

/// The facts of the file as a whole that need no copy.
fn facts(pdf: &Pdf, out: &mut Out) {
    let facts = &mut out.doc.properties.facts;
    facts.pages = Some(pdf.page_count());
    facts.tagged = Some(pdf.tagged());
    facts.encrypted = Some(pdf.encryption().is_some());
}

/// A text value of the Info dictionary or XMP, trimmed; `None` when empty.
fn given(v: Option<&str>) -> Option<&str> {
    v.map(str::trim).filter(|v| !v.is_empty())
}

/// The ISO form of a date, if it parses.
fn iso(d: Option<&PdfDate>) -> Option<&str> {
    d.and_then(|d| d.iso.as_deref())
}

/// Properties from the Info dictionary, else from XMP; an XMP value that differs from the Info
/// value it stands beside is kept as `xmp_<name>`. Then the language, version, PDF/A and
/// PDF/UA parts and the layers.
fn properties(pdf: &Pdf, kept: &Kept<'_>, out: &mut Out) {
    let info: Info = pdf.info();
    let xmp = pdf.xmp();
    let x = |key: &str| given(xmp.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()));
    // A date in the Info dictionary that does not parse, with no XMP date to stand in.
    for (name, d, key) in [
        ("creation", info.created.as_ref(), "xmp:CreateDate"),
        ("modification", info.modified.as_ref(), "xmp:ModifyDate"),
    ] {
        if let Some(d) = d
            && d.iso.is_none()
            && x(key).is_none()
        {
            out.loss(
                "metadata",
                format!(
                    "the {name} date of the document information is not a PDF date: {:?}",
                    d.raw
                ),
                LossReason::Unsupported,
                Severity::Info,
            );
        }
    }
    let mut left_out: Vec<&str> = Vec::new();
    let mut why: Option<String> = None;
    let props = &mut out.doc.properties;
    // The Info value, else the XMP value at `key`; and the XMP value when it differs.
    let pick = |info: Option<&str>, key: &str| -> (Option<String>, Option<String>) {
        let info = given(info);
        let xmp = x(key);
        match (info, xmp) {
            (Some(i), Some(v)) if i != v => (Some(i.to_string()), Some(v.to_string())),
            (Some(i), _) => (Some(i.to_string()), None),
            (None, v) => (v.map(str::to_string), None),
        }
    };
    let mut keep = |name: &'static str, texts: &[&str]| -> bool {
        match kept.take(1, texts) {
            Ok(()) => true,
            Err(w) => {
                why.get_or_insert(w);
                left_out.push(name);
                false
            }
        }
    };
    let mut custom: Vec<(&str, String)> = Vec::new();
    let (title, xmp_title) = pick(info.title.as_deref(), "dc:title");
    if let Some(t) = title
        && keep("title", &[&t])
    {
        props.title = Some(t);
    }
    // The Info author is one name; the XMP creators are a list, joined with "; ".
    let (author, xmp_author) = pick(info.author.as_deref(), "dc:creator");
    if let Some(a) = author {
        let names: Vec<String> = if given(info.author.as_deref()).is_some() {
            vec![a]
        } else {
            a.split("; ")
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_string)
                .collect()
        };
        let texts: Vec<&str> = names.iter().map(String::as_str).collect();
        if keep("authors", &texts) {
            props.authors = names;
        }
    }
    let (keywords, xmp_keywords) = pick(info.keywords.as_deref(), "pdf:Keywords");
    if let Some(k) = keywords {
        // Some producers write the whole list in quotes.
        let list = k
            .strip_prefix('"')
            .and_then(|k| k.strip_suffix('"'))
            .unwrap_or(&k);
        let words: Vec<String> = list
            .split([',', ';'])
            .map(str::trim)
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect();
        let texts: Vec<&str> = words.iter().map(String::as_str).collect();
        if keep("keywords", &texts) {
            props.keywords = words;
        }
    }
    let dates = [
        ("created", iso(info.created.as_ref()), "xmp:CreateDate"),
        ("modified", iso(info.modified.as_ref()), "xmp:ModifyDate"),
    ];
    for (name, from_info, key) in dates {
        let (date, xmp_date) = pick(from_info, key);
        if let Some(d) = date
            && keep(name, &[&d])
        {
            let value = Some(Value::DateTime { value: d });
            match name {
                "created" => props.created = value,
                _ => props.modified = value,
            }
        }
        if let Some(v) = xmp_date {
            custom.push((
                if name == "created" {
                    "xmp_create_date"
                } else {
                    "xmp_modify_date"
                },
                v,
            ));
        }
    }
    let pairs = [
        (
            info.subject.as_deref(),
            "dc:description",
            "subject",
            "xmp_description",
        ),
        (
            info.creator.as_deref(),
            "xmp:CreatorTool",
            "creator",
            "xmp_creator_tool",
        ),
        (
            info.producer.as_deref(),
            "pdf:Producer",
            "producer",
            "xmp_producer",
        ),
    ];
    for (from_info, key, name, xmp_name) in pairs {
        let (v, xv) = pick(from_info, key);
        if let Some(v) = v {
            custom.push((name, v));
        }
        if let Some(v) = xv {
            custom.push((xmp_name, v));
        }
    }
    for (name, v) in [
        ("xmp_title", xmp_title),
        ("xmp_creator", xmp_author),
        ("xmp_keywords", xmp_keywords),
    ] {
        if let Some(v) = v {
            custom.push((name, v));
        }
    }
    if let Some(lang) = pdf.language()
        && keep("language", &[&lang])
    {
        props.language = Some(lang);
    }
    custom.push(("pdf_version", pdf.version()));
    if let Some(part) = x("pdfaid:part") {
        let conformance = x("pdfaid:conformance").unwrap_or_default().to_lowercase();
        custom.push(("pdfa", format!("PDF/A-{part}{conformance}")));
    }
    if let Some(part) = x("pdfuaid:part") {
        custom.push(("pdfua", format!("PDF/UA-{part}")));
    }
    let layers = pdf.layers();
    if !layers.is_empty() {
        props.facts.layers = Some(u32::try_from(layers.len()).unwrap_or(u32::MAX));
        let list: Vec<String> = layers
            .iter()
            .map(|l| {
                let name = given(Some(&l.name)).unwrap_or("unnamed layer");
                let shown = if l.visible { "shown" } else { "hidden" };
                format!("{name} ({shown})")
            })
            .collect();
        custom.push(("layers", list.join(", ")));
    }
    for (name, value) in custom {
        if keep(name, &[name, &value]) {
            props.custom.insert(name.to_string(), Value::Text { value });
        }
    }
    if let Some(why) = why {
        out.loss(
            "metadata",
            format!(
                "{} {} not kept: {why}",
                left_out.join(", "),
                is_are(left_out.len())
            ),
            LossReason::Limit,
            Severity::Warning,
        );
    }
}

/// Page `n` (from 1) of `pages`, if it was listed.
fn page_info(pages: &[PageInfo], n: u32) -> Option<&PageInfo> {
    (n as usize)
        .checked_sub(1)
        .and_then(|i| pages.get(i))
        .filter(|p| p.number == n)
}

/// A page as an outline item names it: its label, else its number.
fn page_label(pages: &[PageInfo], n: u32) -> String {
    page_info(pages, n)
        .and_then(|p| p.label.as_deref())
        .filter(|l| !l.is_empty())
        .map_or_else(|| n.to_string(), str::to_string)
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

    fn push(&mut self, level: u8, item: Block) {
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
/// it is never followed.
fn outline(pdf: &Pdf, pages: &[PageInfo], kept: &Kept<'_>, out: &mut Out) {
    let items = pdf.outline();
    if items.is_empty() {
        return;
    }
    let total = items.len();
    let mut nest = Nest::new();
    let mut cut: Option<(usize, String)> = None;
    for (k, item) in items.iter().enumerate() {
        let title = one_line(&item.title);
        let (inlines, charged) = match (item.page, &item.uri) {
            (Some(p), _) => {
                let text = format!("{title}, page {}", page_label(pages, p));
                (vec![Inline::plain(text.clone())], vec![text])
            }
            (None, Some(uri)) => (
                vec![
                    Inline::plain(format!("{title}, link to ")),
                    Inline::Code { text: uri.clone() },
                ],
                vec![title.clone(), uri.clone()],
            ),
            (None, None) => (vec![Inline::plain(title.clone())], vec![title]),
        };
        let texts: Vec<&str> = charged.iter().map(String::as_str).collect();
        // The item, its paragraph and its inlines.
        if let Err(why) = kept.take(2 + inlines.len() as u64, &texts) {
            cut = Some((k + 1, why));
            break;
        }
        nest.push(item.level, Block::paragraph(inlines));
    }
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
fn links(pdf: &Pdf, pages: &[PageInfo], kept: &Kept<'_>, out: &mut Out) {
    let links = pdf.links();
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
    let mut dangling: Option<(usize, u32, String)> = None;
    let mut cut: Option<(usize, u32, String)> = None;
    for (k, link) in links.iter().enumerate() {
        let from = link.page;
        let mut listed = |kind: &str, name: String, detail: String, target: Option<String>| {
            let texts = [kind, name.as_str(), detail.as_str()];
            kept.take(1, &texts)?;
            if let Some(t) = &target {
                kept.take(0, &[t])?;
            }
            out.doc.facets.embedded.push(Embedded {
                kind: kind.into(),
                name,
                detail: Some(detail),
                target,
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
                listed("remote-link", file.clone(), detail, Some(file.clone()))
            }
            LinkTarget::Launch(file) => listed(
                "launch-action",
                file.clone(),
                format!("from page {from}"),
                Some(file.clone()),
            ),
            LinkTarget::Other(action) => listed(
                "link-action",
                action.clone(),
                format!("from page {from}"),
                None,
            ),
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
                "1 internal link names a destination the file does not have ({name:?}); it is \
                 not in the link graph"
            )
        } else {
            format!(
                "{n} internal links name a destination the file does not have (the first is \
                 {name:?}); they are not in the link graph"
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
                "{n} of {} links {} not kept, from the one on page {page} on: {why}",
                links.len(),
                is_are(n)
            ),
            LossReason::Limit,
            Severity::Warning,
        );
    }
}

/// The attachments, listed with type, size and description. With `extract_assets` each one's
/// bytes come back as a blob, within an eighth of the memory limit for all of them; the ones
/// left out get one Limit loss together.
fn attachments(pdf: &Pdf, opts: &ReadOptions, budget: &Budget, kept: &Kept<'_>, out: &mut Out) {
    let files = pdf.embedded_files();
    let mut room = budget.limits().max_memory_bytes / 8;
    // The attachments not extracted for a limit: the first one, why, and how many.
    let mut left_out: Option<(usize, String, usize)> = None;
    for (i, f) in files.iter().enumerate() {
        let mut parts: Vec<String> = Vec::new();
        if let Some(m) = given(f.mime.as_deref()) {
            parts.push(m.to_string());
        }
        if let Some(n) = f.size {
            parts.push(format!("{n} bytes"));
        }
        if let Some(d) = given(f.description.as_deref()) {
            parts.push(d.to_string());
        }
        let detail = parts.join(", ");
        if let Err(why) = kept.take(1, &[&f.name, &detail]) {
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
        out.doc.facets.embedded.push(Embedded {
            kind: "attachment".into(),
            name: f.name.clone(),
            detail: (!detail.is_empty()).then_some(detail),
            target: None,
            text: None,
        });
        if opts.extract_assets
            && let Err(why) = extract(
                pdf,
                i,
                &f.name,
                given(f.mime.as_deref()),
                &mut room,
                kept,
                out,
            )
        {
            match &mut left_out {
                Some((_, _, n)) => *n += 1,
                None => left_out = Some((i, why, 1)),
            }
        }
    }
    if let Some((i, why, n)) = left_out {
        let name = &files[i].name;
        let what = if n == 1 {
            format!("{name:?} is not extracted: {why}")
        } else {
            format!(
                "{n} attachments are not extracted, the first {name:?} (attachment {}): {why}",
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
            "it unpacks to {n} bytes, more than the {room} bytes left for attachments (an \
             eighth of the memory limit)"
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
                format!("{name:?}: {}", issue.what),
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
        let name =
            given(t.title.as_deref()).map_or_else(|| format!("thread {}", k + 1), str::to_string);
        let detail = page_list(&t.pages);
        if let Err(why) = kept.take(1, &[&name, &detail]) {
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
            name,
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

/// One timeline event per revision, and their count as a fact. Older revisions over the cap
/// are counted, not compared: that is said once as a Limit Info loss.
fn revisions(pdf: &Pdf, budget: &Budget, kept: &Kept<'_>, out: &mut Out) {
    let stop = || budget.check_time().is_err();
    let r = pdf.revisions(&stop);
    let count = r.list.len() + r.skipped;
    if count > 0 {
        out.doc.properties.facts.revisions = Some(u32::try_from(count).unwrap_or(u32::MAX));
    }
    for rev in &r.list {
        let text = revision_text(rev);
        let time = iso(rev.modified.as_ref()).map(str::to_string);
        if let Err(why) = kept.take(1, &[&text, time.as_deref().unwrap_or_default()]) {
            out.loss(
                "revisions",
                format!(
                    "the events of revisions {} to {} are not kept: {why}",
                    rev.number,
                    r.list.last().map_or(rev.number, |l| l.number)
                ),
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
    for issue in &r.issues {
        let (why, mut severity) = reason(issue.kind);
        // The revisions over the cap: counted, and the newest are compared.
        if r.skipped > 0 && issue.kind == IssueKind::Limit && issue.what.contains("only the newest")
        {
            severity = Severity::Info;
        }
        out.loss(location(issue), issue.what.clone(), why, severity);
    }
}

/// The loss reason and severity of an issue kind.
fn reason(kind: IssueKind) -> (LossReason, Severity) {
    match kind {
        IssueKind::Damaged => (LossReason::Damaged, Severity::Warning),
        IssueKind::Limit => (LossReason::Limit, Severity::Warning),
        IssueKind::Unsupported => (LossReason::Unsupported, Severity::Info),
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
