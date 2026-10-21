// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! PDF reader: the text of every page in reading order, with headings, lists, tables,
//! running headers and footers, and the text nobody sees on the page kept apart.
//!
//! pdfparts opens the file with its stream guard and gives the object layer: pages, labels
//! and the structure tree. hayro-interpret reads the text with positions from pdfparts'
//! canonical bytes, never from the file itself, and only for pages the guard let through.
//! Scripts and actions are never run, links are never followed.

mod layout;
mod open;
mod order;
mod tables;
mod text;

#[cfg(test)]
#[path = "../tests/common/mod.rs"]
mod fixtures;

use std::collections::{HashMap, HashSet};

use pdfparts::{IssueKind, PageInfo, Pdf};
use rezzer_limits::Budget;
use rezzer_model::{
    Anchor, Block, CalloutKind, Completeness, Document, FormatId, Inline, Loss, LossReason,
    ReadOptions, ReadOutcome, Reader, ReaderInfo, SectionKind, Severity, Source, StatusColor,
    source_info,
};

use layout::{Context, Cut, Drawn, LinkRoom, PageText, Run, Style, TableRef};
use tables::Tables;
use text::{Caps, Extracted, Fail, Failure, Note, PageItems};

/// Reads PDF files.
#[derive(Debug, Clone, Copy, Default)]
pub struct PdfReader;

impl Reader for PdfReader {
    fn id(&self) -> &'static str {
        "pdf"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn formats(&self) -> &'static [FormatId] {
        &[FormatId::Pdf]
    }

    fn read(&self, src: &Source, opts: &ReadOptions, budget: &Budget) -> ReadOutcome {
        read_pdf(self, src, opts, budget, &|| budget.check_time().is_err())
    }
}

/// Reads a file; `text_stop` is asked before each page and while a page is read.
fn read_pdf(
    reader: &PdfReader,
    src: &Source,
    opts: &ReadOptions,
    budget: &Budget,
    text_stop: &dyn Fn() -> bool,
) -> ReadOutcome {
    let mut info = source_info(src);
    info.reader = Some(ReaderInfo {
        id: reader.id().to_string(),
        version: reader.version().to_string(),
    });
    let out = Out::new(Document::new(info));
    let (pdf, mut out) = match open::open(src.bytes(), opts.password.as_deref(), budget, out) {
        Ok(opened) => opened,
        Err(outcome) => return *outcome,
    };
    out.doc.properties.facts.pages = Some(pdf.page_count());
    out.doc.properties.facts.tagged = Some(pdf.tagged());
    let stop = || budget.check_time().is_err();
    let pages = pdf.pages(&stop);
    let extracted = extract(&pdf, &pages, budget, &mut out, text_stop);
    lay_out(&pdf, &pages, extracted, budget, &mut out);
    issues(&pdf, &mut out);
    out.finish()
}

/// Reads the text of the pages the guard let through.
fn extract(
    pdf: &Pdf,
    pages: &[PageInfo],
    budget: &Budget,
    out: &mut Out,
    stop: &dyn Fn() -> bool,
) -> Extracted {
    if pdf.unsafe_page_tree() {
        out.loss(
            "document",
            "text not read on any page: the page tree has a loop, a shared node or an untyped \
             node, so the text parser could number the pages differently",
            LossReason::Damaged,
            Severity::Error,
        );
        return Extracted::default();
    }
    // pdfparts writes refused streams empty, so the other pages are safe to read; the refused
    // ones keep the guard's loss.
    let refused: HashSet<u32> = pdf.refused_pages().iter().copied().collect();
    let wanted: Vec<u32> = pages
        .iter()
        .map(|p| p.number)
        .filter(|n| !refused.contains(n))
        .collect();
    let caps = Caps::new(
        open::limits(budget).max_items,
        budget.limits().max_memory_bytes,
    );
    text::pages(pdf, &wanted, caps, stop)
}

/// The page marker text: `Page 3`, or `Page iii (3)` when the page has its own label.
fn marker(page: &PageInfo) -> String {
    let n = page.number;
    match page.label.as_deref() {
        Some(label) if !label.is_empty() && label != n.to_string() => {
            format!("Page {label} ({n})")
        }
        _ => format!("Page {n}"),
    }
}

/// The height of the page in the frame of its runs, `None` when the frame was turned.
fn frame_height(page: &PageInfo, items: Option<&PageItems>) -> Option<f32> {
    (!items.is_some_and(|i| i.turned)).then_some(page.height)
}

/// One page section per page, with the laid out text. Pages already read are laid out even
/// when a stop cut the reading: layout is bounded by the element and text budgets.
fn lay_out(pdf: &Pdf, pages: &[PageInfo], extracted: Extracted, budget: &Budget, out: &mut Out) {
    let Extracted {
        pages: read,
        failed,
        stopped_at,
        kept_left,
    } = extracted;
    let info_of: HashMap<u32, &PageInfo> = pages.iter().map(|p| (p.number, p)).collect();
    let texts: Vec<PageText<'_>> = read
        .iter()
        .filter_map(|items| {
            let info = info_of.get(&items.page)?;
            Some(PageText {
                page: items.page,
                height: frame_height(info, Some(items)),
                runs: &items.runs,
            })
        })
        .collect();
    let style = Style::new(&texts);
    drop(texts);
    let mut read: HashMap<u32, PageItems> = read.into_iter().map(|p| (p.page, p)).collect();
    let tree = if pdf.tagged() {
        pdf.struct_tree()
    } else {
        None
    };
    let tables = Tables::default();
    // The model's link targets share what is left of the bytes the pages could keep.
    let links = LinkRoom::new(kept_left);
    let last = pages.last().map_or(0, |p| p.number);
    let mut halted = false;
    // Where each tagged table stands: the index of its page section in the body, of the
    // table among the section's blocks, and its anchor.
    let mut placed: HashMap<usize, (usize, usize, String)> = HashMap::new();
    let widgets = widget_boxes(pdf);
    for info in pages {
        let n = info.number;
        // The section and its marker.
        if let Err(hit) = budget.add_elements(2) {
            // A page that hit the limit already said so.
            if !halted {
                out.loss(
                    format!("p{n}"),
                    format!("{hit}: {} not read", pages_from(n, last)),
                    LossReason::Limit,
                    Severity::Warning,
                );
            }
            break;
        }
        let mut blocks = vec![Block::Paragraph {
            inlines: vec![Inline::Status {
                text: marker(info),
                color: StatusColor::Grey,
            }],
            role: Some("page".into()),
            anchor: None,
        }];
        if let (false, Some(mut items)) = (halted, read.remove(&n)) {
            let roles = |mcid: u32| pdf.role_of(n, mcid);
            let cx = Context {
                style: &style,
                tree,
                roles: &roles,
                budget,
                tables: &tables,
                links: &links,
            };
            if items.links_cut {
                out.loss(
                    format!("p{n}"),
                    "some URI links are not attached to their text: the page has too many \
                     links to match",
                    LossReason::Limit,
                    Severity::Warning,
                );
            }
            notes(n, &items, out);
            let height = frame_height(info, Some(&items));
            let (ox, oy) = items.origin;
            let fields: Vec<[f32; 4]> = widgets
                .get(&n)
                .map(|boxes| {
                    boxes
                        .iter()
                        .map(|b| [b[0] - ox, b[1] - oy, b[2] - ox, b[3] - oy])
                        .collect()
                })
                .unwrap_or_default();
            let drawn = Drawn {
                rules: &items.rules,
                images: &items.images,
                widgets: &fields,
                width: info.width,
                body_size: style.body_size,
            };
            let laid = layout::page_blocks(n, height, std::mem::take(&mut items.runs), &drawn, &cx);
            for (what, why, severity) in laid.notes {
                out.loss(format!("p{n}"), what, why, severity);
            }
            let section = out.doc.body.len();
            let first_table = blocks.len();
            let nothing = laid.blocks.is_empty() && laid.continued.is_empty();
            for (table, rows) in laid.continued {
                if let Some((s, b, _)) = placed.get(&table)
                    && let Some(Block::Section { blocks, .. }) = out.doc.body.get_mut(*s)
                    && let Some(Block::Table { rows: kept, .. }) = blocks.get_mut(*b)
                {
                    kept.extend(rows);
                }
            }
            let mut tagged = laid.tagged.into_iter();
            for (k, block) in laid.blocks.iter().enumerate() {
                if let Block::Table { anchor, .. } = block
                    && let Some(table) = tagged.next().flatten()
                {
                    let anchor = anchor.as_ref().map_or_else(String::new, |a| a.0.clone());
                    placed.insert(table, (section, first_table + k, anchor));
                }
            }
            blocks.extend(laid.blocks);
            // What a stop cuts: the page from the first part not built on, or its hidden text.
            let mut stop = laid.stop.map(|hit| {
                let from = (!nothing).then(|| match laid.cut.unwrap_or(Cut::Block(1)) {
                    Cut::Block(k) => format!("block {k} on"),
                    Cut::Table(k) => format!("table {k} on"),
                    Cut::Item { block, item } => format!("item {item} of list p{n}:b{block} on"),
                    Cut::Row { table, row } => match table {
                        TableRef::Here(k) => format!("row {row} of table p{n}:t{k} on"),
                        TableRef::Earlier(t) => match placed.get(&t) {
                            Some((s, b, anchor)) => {
                                let before = match out.doc.body.get(*s) {
                                    Some(Block::Section { blocks, .. }) => match blocks.get(*b) {
                                        Some(Block::Table { rows, .. }) => rows.len(),
                                        _ => 0,
                                    },
                                    _ => 0,
                                };
                                format!("row {} of table {anchor} on", before + 1)
                            }
                            None => format!("row {row} of a table begun on an earlier page on"),
                        },
                    },
                });
                let what = match (from, pages_after(n, last)) {
                    (None, _) => format!("{} not read", pages_from(n, last)),
                    (Some(from), None) => format!("page {n} from {from} is not read"),
                    (Some(from), Some(later)) => {
                        format!("page {n} from {from} and {later} not read")
                    }
                };
                (hit, what)
            });
            if stop.is_none() {
                match invisible(n, &items, budget, out) {
                    Ok(Some(callout)) => blocks.push(callout),
                    Ok(None) => {}
                    Err(hit) => {
                        let what = match pages_after(n, last) {
                            None => format!("the hidden text of page {n} is not read"),
                            Some(later) => {
                                format!("the hidden text of page {n} and {later} not read")
                            }
                        };
                        stop = Some((hit, what));
                    }
                }
            }
            if let Some((hit, what)) = stop {
                out.loss(
                    format!("p{n}"),
                    format!("{hit}: {what}"),
                    LossReason::Limit,
                    Severity::Warning,
                );
                halted = true;
            }
        }
        out.doc.body.push(Block::Section {
            kind: SectionKind::Page,
            title: None,
            anchor: Some(Anchor(format!("p{n}"))),
            blocks,
        });
    }
    if let Some((count, first, last)) = links.cut() {
        let (s, them) = if count == 1 {
            ("", "its text is")
        } else {
            ("s", "their text is")
        };
        let pages = if first == last {
            format!("page {first}")
        } else {
            format!("pages {first} to {last}")
        };
        out.loss(
            format!("p{first}"),
            format!(
                "the targets of {count} link{s} on {pages} are not kept: the copies of link \
                 targets take more than the {} bytes left of what the file may keep; {them} \
                 kept without the link",
                links.room()
            ),
            LossReason::Limit,
            Severity::Warning,
        );
    }
    failures(&failed, out);
    if let Some(first) = stopped_at {
        let last = pages.last().map_or(first, |p| p.number);
        out.loss(
            format!("p{first}"),
            format!("text of pages {first} to {last} not read: the time limit was reached"),
            LossReason::Limit,
            Severity::Warning,
        );
    }
    if pages.len() < pdf.page_count() as usize {
        out.loss(
            "document",
            format!(
                "pages {} to {} not read: the time limit was reached",
                pages.len() + 1,
                pdf.page_count()
            ),
            LossReason::Limit,
            Severity::Warning,
        );
    }
}

/// Pages `first` to `last` as the subject of "not read": "page 3 is", "pages 3 to 9 are".
fn pages_from(first: u32, last: u32) -> String {
    if first >= last {
        format!("page {first} is")
    } else {
        format!("pages {first} to {last} are")
    }
}

/// The pages after `page` up to `last`, if any, as in [`pages_from`].
fn pages_after(page: u32, last: u32) -> Option<String> {
    (page < last).then(|| pages_from(page + 1, last))
}

/// One loss per run of consecutive pages that failed the same way.
fn failures(failed: &[Failure], out: &mut Out) {
    let mut i = 0;
    while i < failed.len() {
        let first = &failed[i];
        let mut j = i + 1;
        while j < failed.len()
            && failed[j].page == failed[j - 1].page + 1
            && failed[j].why == first.why
            && failed[j].kind == first.kind
        {
            j += 1;
        }
        let last = failed[j - 1].page;
        let pages = if last == first.page {
            format!("page {}", first.page)
        } else {
            format!("pages {} to {last}", first.page)
        };
        let why = &first.why;
        let (what, reason) = match first.kind {
            Fail::Damaged => (
                format!("text of {pages} not read: the text parser failed: {why}"),
                LossReason::Damaged,
            ),
            Fail::Limit => (
                format!("text of {pages} not read: {why}"),
                LossReason::Limit,
            ),
            Fail::Internal => (
                format!(
                    "text of {pages} not read: the text parser stopped with an internal error \
                     while {why}"
                ),
                LossReason::Internal,
            ),
        };
        out.loss(format!("p{}", first.page), what, reason, Severity::Warning);
        i = j;
    }
}

/// What the text parser could not read on a page, and the glyphs without a Unicode mapping.
fn notes(page: u32, items: &PageItems, out: &mut Out) {
    for note in &items.notes {
        let (what, why, severity) = match note {
            Note::Limit(what) => (what, LossReason::Limit, Severity::Warning),
            Note::Damaged(what) => (what, LossReason::Damaged, Severity::Warning),
            Note::Unsupported(what) => (what, LossReason::Unsupported, Severity::Info),
        };
        out.loss(format!("p{page}"), what.clone(), why, severity);
    }
    if items.unmapped > 0 {
        let n = items.unmapped;
        let glyphs = if n == 1 { "glyph" } else { "glyphs" };
        out.loss(
            format!("p{page}"),
            format!("{n} {glyphs} without a Unicode mapping on page {page}"),
            LossReason::Unsupported,
            Severity::Info,
        );
    }
}

/// The lines of runs: the runs of one line joined.
fn run_lines(runs: &[Run]) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut last = None;
    for r in runs {
        if last == Some(r.line) {
            if let Some(l) = lines.last_mut() {
                l.push_str(&r.text);
            }
        } else {
            lines.push(r.text.trim_start().to_string());
        }
        last = Some(r.line);
    }
    lines
}

/// The folded callout with the text that puts no glyphs on the page and the text of
/// switched-off layers, each layer under its name, and their Info losses.
fn invisible(
    page: u32,
    items: &PageItems,
    budget: &Budget,
    out: &mut Out,
) -> Result<Option<Block>, rezzer_limits::LimitHit> {
    let (runs, layer) = (&items.invisible, &items.hidden_layer);
    if runs.is_empty() && layer.is_empty() && items.hidden_layers.is_empty() {
        return Ok(None);
    }
    let mut lines = run_lines(runs);
    lines.extend(layer.iter().cloned());
    for (name, layer_runs) in &items.hidden_layers {
        lines.push(format!("Hidden layer {name}:"));
        lines.extend(run_lines(layer_runs));
    }
    budget.add_elements(1 + 2 * lines.len() as u64)?;
    let mut blocks = Vec::with_capacity(lines.len());
    for l in lines {
        budget.charge_text(l.len())?;
        let l = l.trim_end().to_string();
        if !l.is_empty() {
            blocks.push(Block::paragraph(vec![Inline::plain(l)]));
        }
    }
    let mut said = Vec::new();
    if !runs.is_empty() {
        let n = runs.len();
        let word = if n == 1 { "run" } else { "runs" };
        said.push(format!("{n} {word} of invisible text (render mode 3 or 7)"));
    }
    if !layer.is_empty() {
        said.push(format!(
            "an invisible text layer (render mode 3) of {} lines",
            layer.len()
        ));
    }
    let shown = "shown in a folded callout at the end of the page, not in the text";
    if !said.is_empty() {
        out.loss(
            format!("p{page}"),
            format!("{}: {shown}", said.join(" and ")),
            LossReason::Unsupported,
            Severity::Info,
        );
    }
    for (name, layer_runs) in &items.hidden_layers {
        let n = layer_runs.len();
        let word = if n == 1 { "run" } else { "runs" };
        out.loss(
            format!("p{page}"),
            format!("{n} {word} in hidden layer {name}: {shown}"),
            LossReason::Unsupported,
            Severity::Info,
        );
    }
    Ok(Some(Block::Callout {
        kind: CalloutKind::Details,
        title: Some("Invisible text on this page".into()),
        folded: Some(true),
        blocks,
        anchor: None,
    }))
}

/// The boxes of the form fields of each page, in user space.
fn widget_boxes(pdf: &Pdf) -> HashMap<u32, Vec<[f32; 4]>> {
    let mut out: HashMap<u32, Vec<[f32; 4]>> = HashMap::new();
    for (page, b) in pdf.widget_boxes() {
        out.entry(page).or_default().push(b);
    }
    out
}

/// The issues pdfparts found, as losses. `page 3` becomes the page anchor `p3`.
fn issues(pdf: &Pdf, out: &mut Out) {
    for issue in pdf.issues() {
        let location = match issue.location.strip_prefix("page ") {
            Some(n) if n.parse::<u32>().is_ok() => format!("p{n}"),
            _ => issue.location.clone(),
        };
        let (why, severity) = match issue.kind {
            IssueKind::Damaged => (LossReason::Damaged, Severity::Warning),
            IssueKind::Limit => (LossReason::Limit, Severity::Warning),
            IssueKind::Unsupported => (LossReason::Unsupported, Severity::Info),
        };
        out.loss(location, issue.what, why, severity);
    }
}

/// The document being built, with whether something was not read.
pub(crate) struct Out {
    pub doc: Document,
    pub partial: bool,
}

impl Out {
    fn new(doc: Document) -> Out {
        Out {
            doc,
            partial: false,
        }
    }

    /// Adds a loss. Damaged parts, limits and parts not read make the read partial; an
    /// unsupported detail or an uncertain fact does not.
    pub fn loss(
        &mut self,
        location: impl Into<String>,
        what: impl Into<String>,
        why: LossReason,
        severity: Severity,
    ) {
        if matches!(
            why,
            LossReason::Damaged
                | LossReason::Limit
                | LossReason::Encrypted
                | LossReason::NotImplemented
                | LossReason::Internal
        ) {
            self.partial = true;
        }
        self.doc.losses.push(Loss {
            location: location.into(),
            what: what.into(),
            why,
            severity,
        });
    }

    pub fn finish(self) -> ReadOutcome {
        ReadOutcome {
            blobs: Vec::new(),
            document: self.doc,
            completeness: if self.partial {
                Completeness::Partial
            } else {
                Completeness::Complete
            },
        }
    }

    pub fn failed(self) -> ReadOutcome {
        ReadOutcome {
            blobs: Vec::new(),
            document: self.doc,
            completeness: Completeness::Failed,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rezzer_limits::Limits;
    use rezzer_model::{Bytes, Confidence, Detection};

    use super::*;
    use crate::fixtures::{Page, pdf, show};

    #[test]
    fn failures_are_grouped_by_run_of_pages() {
        let mut out = Out::new(Document::new(source_info(&Source {
            path: None,
            name: "f.pdf".into(),
            bytes: Bytes::Owned(Arc::new(Vec::new())),
            sha256: "00".repeat(32),
            size: 0,
            format: FormatId::Pdf,
            detection: Detection {
                confidence: Confidence::Certain,
                evidence: "x".into(),
            },
            modified: None,
        })));
        let bad = |page: u32, why: &str| Failure {
            page,
            why: why.to_string(),
            kind: Fail::Damaged,
        };
        failures(
            &[
                bad(1, "a"),
                bad(2, "a"),
                bad(3, "a"),
                bad(5, "a"),
                bad(6, "b"),
            ],
            &mut out,
        );
        let said: Vec<(&str, &str)> = out
            .doc
            .losses
            .iter()
            .map(|l| (l.location.as_str(), l.what.as_str()))
            .collect();
        assert_eq!(
            said,
            [
                (
                    "p1",
                    "text of pages 1 to 3 not read: the text parser failed: a"
                ),
                ("p5", "text of page 5 not read: the text parser failed: a"),
                ("p6", "text of page 6 not read: the text parser failed: b"),
            ]
        );
        assert!(out.partial);
    }

    #[test]
    fn time_stop_between_pages_is_partial() {
        let page = Page::text(show("F1", 12.0, 72.0, 700.0, "Some words."));
        let bytes = pdf(&std::iter::repeat_n(page, 70).collect::<Vec<_>>(), "");
        let src = Source {
            path: None,
            name: "file.pdf".into(),
            bytes: Bytes::Owned(Arc::new(bytes.clone())),
            sha256: "00".repeat(32),
            size: bytes.len() as u64,
            format: FormatId::Pdf,
            detection: Detection {
                confidence: Confidence::Certain,
                evidence: "%PDF-".into(),
            },
            modified: None,
        };
        let asked = std::cell::Cell::new(0);
        let budget = Budget::new(Limits::default());
        // The time runs out after 64 pages: from then on the budget says so too.
        let stop = || {
            asked.set(asked.get() + 1);
            if asked.get() > 64 {
                budget.cancel();
            }
            budget.is_cancelled()
        };
        let out = read_pdf(&PdfReader, &src, &ReadOptions::default(), &budget, &stop);
        assert_eq!(out.completeness, Completeness::Partial);
        let sections = out
            .document
            .body
            .iter()
            .filter(|b| matches!(b, Block::Section { .. }))
            .count();
        assert_eq!(sections, 70);
        let loss = out
            .document
            .losses
            .iter()
            .find(|l| l.why == LossReason::Limit)
            .expect("a limit loss");
        assert_eq!(loss.location, "p65");
        assert!(loss.what.contains("pages 65 to 70"), "{loss:?}");
        let with_text = out
            .document
            .body
            .iter()
            .filter(|b| matches!(b, Block::Section { blocks, .. } if blocks.len() > 1))
            .count();
        assert_eq!(with_text, 64);
    }
}
