// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! PDF reader: the text of every page in reading order, with headings, lists, tables,
//! running headers and footers, and the text nobody sees on the page kept apart.
//!
//! pdfparts opens the file with its stream guard and gives the object layer: pages, labels
//! and the structure tree. hayro-interpret reads the text with positions from pdfparts'
//! canonical bytes, never from the file itself, and only for pages the guard let through.
//! `meta.rs` and `objects.rs` add the metadata, outline, links, attachments, threads and
//! revisions, and `forms.rs` the form logic, scripts and signatures; `quote.rs` quotes the
//! text under annotations and `annots.rs` puts them at their passage. `render.rs` lists the
//! pages that need vision and renders them on request; `fallback.rs` reads the text of a
//! file pdfparts cannot open with the same text walk.
//! Scripts and actions are never run, links are never followed.

mod annots;
mod barfont;
mod codes;
mod engine;
mod fallback;
mod forms;
mod found;
mod layout;
mod marks;
mod meta;
mod objects;
mod open;
mod order;
mod quote;
mod regions;
mod render;
mod scan;
mod symbol;
mod tables;
mod text;
mod work;

#[cfg(test)]
#[path = "../tests/common/mod.rs"]
mod fixtures;
/// The test fixtures name this crate as the integration tests do.
#[cfg(test)]
extern crate self as rezzer_read_pdf;

use std::collections::{HashMap, HashSet};

use pdfparts::{Annot, PageInfo, Pdf};
use rezzer_limits::Budget;
use rezzer_model::{
    Anchor, Blob, Block, CalloutKind, Cell, Completeness, Document, FormatId, Inline, Loss,
    LossReason, ReadOptions, ReadOutcome, Reader, ReaderInfo, SectionKind, Severity, Source,
    StatusColor, source_info,
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
        read_timed(src, opts, budget, None)
    }
}

/// The steps of work of the reads on this thread so far, the region engine's included (see
/// `work`); `None` in a release build. For the work bound tests; not a stable interface.
#[doc(hidden)]
pub use work::work_steps;

/// The region engine the regions of [`read_with_regions`] come from, for its callers.
#[doc(hidden)]
pub use layoutregions;

/// [`PdfReader::read`] with the regions the layout found on each page laid out, in page
/// order. For the region report and the tests; not a stable interface.
#[doc(hidden)]
pub fn read_with_regions(
    src: &Source,
    opts: &ReadOptions,
    budget: &Budget,
) -> (ReadOutcome, Vec<layoutregions::PageRegions>) {
    let (out, pages) = read_engine_pages(src, opts, budget);
    (out, pages.into_iter().map(|p| p.regions).collect())
}

/// One page laid out, as the region engine took it and with the regions found on it.
#[doc(hidden)]
#[derive(Debug)]
pub struct EnginePage {
    /// The page's primitives: `PrimId::Text(i)` of a region is `input.texts[i]`.
    pub input: layoutregions::PageIn,
    pub regions: layoutregions::PageRegions,
    /// The anchor of the block at each place a region's `host` names, if it has one.
    pub anchors: Vec<Option<String>>,
    /// What the block each run (by its id) was written in is (see [`block_word`]), `None` for
    /// a run written in no block of the page.
    pub run_blocks: Vec<Option<&'static str>>,
    /// The place among `block_words` of the block each run (by its id) was written in, `None`
    /// for a run written in no block of the page.
    pub run_block_at: Vec<Option<usize>>,
    /// The layout of the page stopped at a limit before it wrote every block.
    pub stopped: bool,
    /// What each block of the page is (see [`block_word`]), by its place: a region's `host`
    /// names one of them, or a place past them for rows of a table begun on an earlier page.
    pub block_words: Vec<&'static str>,
    /// The file is read by its tag tree (its footnotes are the tags'), as the facts of the
    /// file are learned.
    pub tagged: bool,
}

/// What a block of a page is, in one word: its type, and for a paragraph with a role of the
/// page (`page-header`, `page-footer`, `footnote`) that role.
#[doc(hidden)]
pub fn block_word(block: &Block) -> &'static str {
    match block {
        Block::Heading { .. } => "heading",
        Block::List { .. } => "list",
        Block::Table { .. } => "table",
        Block::Spatial { .. } => "spatial",
        Block::Section { .. } => "section",
        Block::Media { .. } => "media",
        Block::Paragraph { role, .. } => match role.as_deref() {
            Some("page-header") => "page-header",
            Some("page-footer") => "page-footer",
            Some("footnote") => "footnote",
            _ => "paragraph",
        },
        _ => "other",
    }
}

/// The pages of a read as the region engine laid them out, as JSON, for the measures over
/// dumped reads: per page its number, whether it stopped, the runs in no region and in two,
/// and per region its kind, its host's anchor and word (see [`block_word`]; `null` for a host
/// that is no block of the page) and, per word, how many of its runs with text were written in
/// such a block (`none` for no block). Not a stable interface.
#[doc(hidden)]
pub fn engine_report(pages: &[EnginePage]) -> String {
    let quote = |s: &str| {
        let mut q = String::from("\"");
        for c in s.chars() {
            match c {
                '"' => q.push_str("\\\""),
                '\\' => q.push_str("\\\\"),
                c if (c as u32) < 0x20 => q.push_str(&format!("\\u{:04x}", c as u32)),
                c => q.push(c),
            }
        }
        q.push('"');
        q
    };
    let mut out = String::from("[");
    for (n, p) in pages.iter().enumerate() {
        let runs = p.input.texts.len();
        let mut count = vec![0u32; runs];
        for r in &p.regions.regions {
            for id in &r.prims {
                if let layoutregions::PrimId::Text(i) = *id
                    && let Some(c) = count.get_mut(i as usize)
                {
                    *c += 1;
                }
            }
        }
        let ids = |want: &dyn Fn(u32) -> bool| -> String {
            let list: Vec<String> = count
                .iter()
                .enumerate()
                .filter(|(_, c)| want(**c))
                .map(|(i, _)| i.to_string())
                .collect();
            format!("[{}]", list.join(","))
        };
        if n > 0 {
            out.push(',');
        }
        let issues = format!(
            "[{}]",
            p.regions
                .issues
                .iter()
                .map(|i| quote(&format!("{:?}", i.kind)))
                .collect::<Vec<_>>()
                .join(",")
        );
        out.push_str(&format!(
            "\n{{\"page\":{},\"runs\":{runs},\"stopped\":{},\"issues\":{},\"uncovered\":{},\"twice\":{},\"regions\":[",
            p.regions.page,
            p.stopped,
            issues,
            ids(&|c| c == 0),
            ids(&|c| c > 1)
        ));
        for (k, r) in p.regions.regions.iter().enumerate() {
            let mut words: std::collections::BTreeMap<&str, u32> =
                std::collections::BTreeMap::new();
            let mut with_text = 0u32;
            for id in &r.prims {
                if let layoutregions::PrimId::Text(i) = *id
                    && let Some(t) = p.input.texts.get(i as usize)
                    && !t.text.trim().is_empty()
                {
                    with_text += 1;
                    let word = p
                        .run_blocks
                        .get(i as usize)
                        .copied()
                        .flatten()
                        .unwrap_or("none");
                    *words.entry(word).or_insert(0) += 1;
                }
            }
            let host = p
                .anchors
                .get(r.host as usize)
                .cloned()
                .flatten()
                .map_or_else(|| "null".to_string(), |a| quote(&a));
            let host_word = p
                .block_words
                .get(r.host as usize)
                .map_or_else(|| "null".to_string(), |w| quote(w));
            let blocks: Vec<String> = words
                .iter()
                .map(|(w, c)| format!("{}:{c}", quote(w)))
                .collect();
            if k > 0 {
                out.push(',');
            }
            out.push_str(&format!(
                "{{\"kind\":{},\"host\":{host},\"host_word\":{host_word},\"runs\":{with_text},\"blocks\":{{{}}}}}",
                quote(r.kind.name()),
                blocks.join(",")
            ));
        }
        out.push_str("]}");
    }
    out.push_str("\n]\n");
    out
}

/// The region engine's decisions on the pages of one file (see [`EnginePage::input`]), made by
/// the layout's own decision step: each as its kind, its primitives and what made its segment
/// (its source and those of the segments over it, innermost first, joined by `<`). `tagged` as
/// for the file ([`EnginePage::tagged`]). For the metamorphic tests; not a stable interface.
#[doc(hidden)]
pub fn engine_decisions(
    pages: &[layoutregions::PageIn],
    tagged: bool,
) -> Vec<Vec<(layoutregions::Kind, Vec<layoutregions::PrimId>, String)>> {
    layout::engine_decisions(pages, tagged)
}

/// The facts of the file of `pages` learned from the runs in the order they have on each page
/// (the reader's flow) and in the order of their places (as [`engine_decisions`] learns
/// them), the runs of the legends named by the ids of `pages` in both. The first is what the
/// read learns for an untagged file; for a tagged file the read learns from more runs (those
/// an ActualText replaces are dropped before the engine pages are made). For the metamorphic
/// tests, which compare the two; not a stable interface.
#[doc(hidden)]
pub fn engine_facts_by_order(
    pages: &[layoutregions::PageIn],
    tagged: bool,
) -> [layoutregions::file::FileFacts; 2] {
    layout::engine_facts_by_order(pages, tagged)
}

/// [`engine_decisions`] with the reasons of each decision in words (its certainty, score and
/// signals, and the candidates of its segment). For the path trace of a metamorphic pin; not a
/// stable interface.
#[doc(hidden)]
pub fn engine_decisions_why(
    pages: &[layoutregions::PageIn],
    tagged: bool,
) -> Vec<Vec<layout::DecisionWhy>> {
    layout::engine_decisions_why(pages, tagged)
}

/// [`read_with_regions`] with the input of each page, so a test can map a region's pieces back
/// to their text and count the page's runs. Not a stable interface.
#[doc(hidden)]
pub fn read_engine_pages(
    src: &Source,
    opts: &ReadOptions,
    budget: &Budget,
) -> (ReadOutcome, Vec<EnginePage>) {
    let mut pages = Vec::new();
    let out = read_timed(src, opts, budget, Some(&mut pages));
    (out, pages)
}

/// The read of [`PdfReader::read`]; each page laid out goes to `regions`, if given.
fn read_timed(
    src: &Source,
    opts: &ReadOptions,
    budget: &Budget,
    regions: Option<&mut Vec<EnginePage>>,
) -> ReadOutcome {
    // The text is read for three quarters of the time limit at most: the pages read must
    // still be laid out, or their text would be lost with the rest.
    // A stop there is a time stop of the read, as one at the limit itself is.
    let text_time = budget.limits().max_time * 3 / 4;
    let text_stop = || {
        if budget.check_time().is_err() {
            return true;
        }
        let near = budget.elapsed() > text_time;
        if near {
            budget.note_time_stop();
        }
        near
    };
    read_pdf(&PdfReader, src, opts, budget, &text_stop, regions)
}

/// Reads a file; `text_stop` is asked before each page and while a page is read. The regions
/// of each page laid out go to `regions`, if given.
fn read_pdf(
    reader: &PdfReader,
    src: &Source,
    opts: &ReadOptions,
    budget: &Budget,
    text_stop: &dyn Fn() -> bool,
    regions: Option<&mut Vec<EnginePage>>,
) -> ReadOutcome {
    // Rendered pages and extracted attachments share one room.
    let blob_room = budget.limits().max_memory_bytes / 8;
    read_pdf_in(reader, src, opts, budget, text_stop, blob_room, regions)
}

/// [`read_pdf`] with `blob_room` bytes for rendered pages and extracted attachments.
fn read_pdf_in(
    reader: &PdfReader,
    src: &Source,
    opts: &ReadOptions,
    budget: &Budget,
    text_stop: &dyn Fn() -> bool,
    blob_room: u64,
    regions: Option<&mut Vec<EnginePage>>,
) -> ReadOutcome {
    let mut info = source_info(src);
    info.reader = Some(ReaderInfo {
        id: reader.id().to_string(),
        version: reader.version().to_string(),
    });
    let mut out = Out::new(Document::new(info));
    out.blob_room = blob_room;
    let (pdf, mut out) = match open::open(src.bytes(), opts.password.as_deref(), budget, out) {
        Ok(opened) => opened,
        Err(open::Refused::Done(outcome)) => return *outcome,
        Err(open::Refused::Unreadable(e, out)) => {
            return fallback::read_file(src, opts, budget, &e, *out, text_stop, regions);
        }
    };
    let stop = || budget.check_time().is_err();
    let pages = pdf.pages(&stop);
    // The revisions are read before any list that draws on pdfparts' budget; the links once,
    // for the text and for the object layer; the annotations before the text, which quotes
    // what they mark.
    let history = objects::revisions(&pdf, budget, &mut out);
    let links = pdf.links();
    let annots = pdf.annotations();
    let marked = quote::marked(&annots);
    let read = Read { src, opts, budget };
    let extracted = extract(&pdf, &pages, &links, &marked, &read, &mut out, text_stop);
    drop(marked);
    let (room, placed, needs, places) = lay_out(
        Some(&pdf),
        &pages,
        extracted,
        &annots,
        budget,
        &mut out,
        regions,
    );
    let scans = render::render_pages(&pdf, &needs, opts, budget, &mut out);
    render::apply_codes(Some(scans), places, &mut out);
    drop(needs);
    annots::read(&annots, placed, budget, &room, &mut out);
    drop(annots);
    objects::read(&pdf, &pages, &links, history, opts, budget, &room, &mut out);
    out.finish()
}

/// What one read is of: the file, its options and its budget.
struct Read<'r> {
    src: &'r Source,
    opts: &'r ReadOptions,
    budget: &'r Budget,
}

/// Reads the text of the pages the guard let through. A page with fonts that gave no text is
/// read again with the fallback parser; its text replaces the empty page, with a loss.
fn extract(
    pdf: &Pdf,
    pages: &[PageInfo],
    links: &[pdfparts::Link],
    marked: &quote::Marked<'_>,
    read: &Read<'_>,
    out: &mut Out,
    stop: &dyn Fn() -> bool,
) -> Extracted {
    let budget = read.budget;
    let caps = Caps::new(
        open::limits(budget).max_items,
        budget.limits().max_memory_bytes,
        pdf.object_bytes(),
    );
    if pdf.unsafe_page_tree() {
        out.loss(
            "document",
            "text not read on any page: the page tree has a loop, a shared or missing node, an \
             untyped node, a node of another type or too much depth, so the text parser could \
             number the pages differently",
            LossReason::Damaged,
            Severity::Error,
        );
        // What the pages may keep stays for the object layer.
        return text::pages(pdf, &[], links, marked, &text::Widgets::new(), caps, stop);
    }
    // pdfparts writes refused streams empty, so the other pages are safe to read; the refused
    // ones keep the guard's loss.
    let refused: HashSet<u32> = pdf.refused_pages().iter().copied().collect();
    let wanted: Vec<u32> = pages
        .iter()
        .map(|p| p.number)
        .filter(|n| !refused.contains(n))
        .collect();
    let widgets = widget_boxes(pdf);
    let mut extracted = text::pages(pdf, &wanted, links, marked, &widgets, caps, stop);
    drop(widgets);
    let fonts: HashSet<u32> = pages
        .iter()
        .filter(|p| p.has_fonts)
        .map(|p| p.number)
        .collect();
    let empty: Vec<u32> = extracted
        .pages
        .iter()
        .filter(|p| fonts.contains(&p.page) && !p.has_text())
        .map(|p| p.page)
        .collect();
    if empty.is_empty() {
        return extracted;
    }
    let again = Caps {
        max_kept: extracted.kept_left,
        ..caps
    };
    let Some(found) = fallback::pages(pdf, read.src, read.opts, budget, &empty, again, stop) else {
        return extracted;
    };
    // What the fallback kept counts against the same bytes; a page it read without text
    // gives its share back, and so does an empty page its text replaces.
    let mut left = found.kept_left;
    for items in found.pages {
        let Some(old) = extracted
            .pages
            .iter_mut()
            .find(|p| p.page == items.page)
            .filter(|_| items.has_text())
        else {
            left += items.kept;
            continue;
        };
        left += old.kept;
        let n = items.page;
        *old = items;
        old.fallback = true;
        out.loss(
            format!("p{n}"),
            format!("page {n}: text read with the fallback parser"),
            LossReason::Uncertain,
            Severity::Warning,
        );
    }
    extracted.kept_left = left;
    // Pages a limit kept the fallback from reading say so. Where the fallback parser failed
    // otherwise, the first read stands: it read the page and found no text.
    let mut failed: Vec<text::Failure> = found
        .failed
        .into_iter()
        .filter(|f| f.kind == text::Fail::Limit)
        .collect();
    if let Some(first) = found.stopped_at {
        let read: HashSet<u32> = extracted.pages.iter().map(|p| p.page).collect();
        failed.extend(
            empty
                .iter()
                .filter(|&&n| n >= first && read.contains(&n))
                .map(|&page| text::Failure {
                    page,
                    why: "the time limit was reached".into(),
                    kind: text::Fail::Limit,
                }),
        );
    }
    if !failed.is_empty() {
        extracted.failed.append(&mut failed);
        extracted.failed.sort_by_key(|f| f.page);
    }
    extracted
}

/// Adds a `found-by` note paragraph to the blocks of page `n`, charged to the budget; past
/// the budget the read says that the note saying `what` is not written.
fn found_by_note(
    blocks: &mut Vec<Block>,
    note: String,
    what: &str,
    n: u32,
    budget: &Budget,
    out: &mut Out,
) {
    let charged = budget
        .add_elements(1)
        .and_then(|()| budget.charge_text(note.len()));
    match charged {
        Ok(()) => blocks.push(Block::Paragraph {
            inlines: vec![Inline::plain(note)],
            role: Some("found-by".into()),
            anchor: None,
        }),
        Err(hit) => out.loss(
            format!("p{n}"),
            format!("{hit}: the note that says {what} is not written"),
            LossReason::Limit,
            Severity::Warning,
        ),
    }
}

/// The line that says how many pictures of page `n` are too small to be listed (`count`),
/// if any; when the limits leave no room for it, a loss says so.
fn very_small_note(n: u32, count: usize, budget: &Budget, out: &mut Out) -> Option<Block> {
    if count == 0 {
        return None;
    }
    let text = if count == 1 {
        "1 very small picture (under 1 cm2) is not listed".to_string()
    } else {
        format!("{count} very small pictures (under 1 cm2) are not listed")
    };
    match budget
        .add_elements(1)
        .and_then(|()| budget.charge_text(text.len()))
    {
        Ok(()) => Some(Block::Paragraph {
            inlines: vec![Inline::Status {
                text,
                color: StatusColor::Grey,
            }],
            role: Some("pictures".into()),
            anchor: None,
        }),
        Err(hit) => {
            out.loss(
                format!("p{n}"),
                format!("{hit}: the count of the very small pictures of page {n} is not written"),
                LossReason::Limit,
                Severity::Warning,
            );
            None
        }
    }
}

/// Reads the separators drawn as marks on `pages` with the style the file's visible text
/// shows (each line of runs scanned whole), before the pages are laid out. A number whose
/// separators are not read is a Warning loss and makes the read partial: it may be off by a
/// factor of 10 or 100. Returns the `found-by` note of each page that got separators.
fn separators(pages: &mut [PageItems], out: &mut Out) -> HashMap<u32, String> {
    // Only visible glyphs get placeholders: they are in the runs and the quotes.
    let placed = |p: &PageItems| {
        p.runs
            .iter()
            .any(|r| r.text.contains(marks::is_placeholder))
            || p.quotes
                .iter()
                .flat_map(|q| &q.text)
                .any(|t| t.contains(marks::is_placeholder))
    };
    if !pages.iter().any(placed) {
        return HashMap::new();
    }
    let (style, votes) = marks::style_in(pages);
    let mut notes = HashMap::new();
    for (page, got) in marks::resolve(pages, &style) {
        for why in &got.unread {
            out.loss(
                format!("p{page}"),
                format!("a separator drawn as a mark on page {page} is not read: {why}"),
                LossReason::Unsupported,
                Severity::Warning,
            );
        }
        if got.more > 0 {
            out.loss(
                format!("p{page}"),
                format!(
                    "separators drawn as marks in {} more numbers on page {page} are not read; \
                     their digits are kept without them",
                    got.more
                ),
                LossReason::Unsupported,
                Severity::Warning,
            );
        }
        // Content Rezzer cannot read: an `Unsupported` loss, which by itself leaves a read
        // complete, so the read is made partial here.
        if !got.unread.is_empty() {
            out.partial = true;
        }
        if let Some(note) = marks::note(&got, &style, &votes) {
            notes.insert(page, note);
        }
    }
    notes
}

/// The page marker text: `Page 3`, or `Page iii (3)` when the page has its own label.
pub(crate) fn marker(page: &PageInfo) -> String {
    let n = page.number;
    match page.label.as_deref() {
        Some(label) if !label.is_empty() && label != n.to_string() => {
            format!("Page {label} ({n})")
        }
        _ => format!("Page {n}"),
    }
}

/// The width and height of the page in the frame of its runs.
fn frame_size(page: &PageInfo, items: Option<&PageItems>) -> (f32, f32) {
    let turn = items.map(|i| i.turn).unwrap_or_default();
    turn.size(page.width, page.height)
}

/// A box in the frame of a page's runs back in its visible box (`/Rotate` not applied, origin
/// at the lower left corner), `w` by `h` points, as [`PageItems`] states its coordinates: a
/// turned frame is turned back, and what reaches past the box (a run's descent at its edge)
/// is clamped to it.
fn page_box(b: layoutregions::Bounds, items: &PageItems, w: f32, h: f32) -> [f32; 4] {
    let b = [b.x0, b.y0, b.x1, b.y1];
    let [x0, y0, x1, y1] = if items.turned {
        items.turn.unplace_box(b, w, h)
    } else {
        b
    };
    [
        x0.clamp(0.0, w),
        y0.clamp(0.0, h),
        x1.clamp(0.0, w),
        y1.clamp(0.0, h),
    ]
}

/// The height of the page in the frame of its runs.
fn frame_height(page: &PageInfo, items: Option<&PageItems>) -> f32 {
    frame_size(page, items).1
}

/// One page section per page, with the laid out text. Pages already read are laid out even
/// when a stop cut the reading: layout is bounded by the element and text budgets. Returns
/// what is left of the bytes the file may keep, for the object layer, and where each of
/// `annots` stands with what it quotes. The regions of each page laid out go to `regions`,
/// if given.
fn lay_out(
    pdf: Option<&Pdf>,
    pages: &[PageInfo],
    extracted: Extracted,
    annots: &[Annot],
    budget: &Budget,
    out: &mut Out,
    mut regions: Option<&mut Vec<EnginePage>>,
) -> LaidOut {
    let Extracted {
        pages: mut read,
        failed,
        stopped_at,
        kept_left,
        quotes_cut,
        quote_visits,
    } = extracted;
    let mut on_page: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, a) in annots.iter().enumerate() {
        on_page.entry(a.page).or_default().push(i);
    }
    let mut spots = vec![annots::Placed::default(); annots.len()];
    let mut unquoted = quote::Unquoted::default();
    let mut found_by = separators(&mut read, out);
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
    let tree = match pdf {
        Some(pdf) if pdf.tagged() => pdf.struct_tree(),
        _ => None,
    };
    // The layout reads a page by its tags only with a structure tree.
    let style = Style::learn(&texts, tree.is_some());
    drop(texts);
    let mut read: HashMap<u32, PageItems> = read.into_iter().map(|p| (p.page, p)).collect();
    // The pages whose text was laid out.
    let mut read_pages: HashSet<u32> = HashSet::new();
    let figures = pdf.map(render::page_figures).unwrap_or_default();
    let furniture = render::furniture(read.values());
    let count = pdf.map_or(pages.len(), |pdf| pdf.page_count() as usize);
    let everywhere = render::on_every_page(read.values(), count);
    // Small pictures by what they are: equal pictures share one asset.
    let mut pictures = Pictures::default();
    let mut needs: Vec<render::Need> = Vec::new();
    let tables = Tables::default();
    // The model's link targets share what is left of the bytes the pages could keep.
    let links = LinkRoom::new(kept_left);
    let last = pages.last().map_or(0, |p| p.number);
    let mut halted = false;
    // Where each tagged table stands: the index of its page section in the body, of the
    // table among the section's blocks, and its anchor.
    let mut placed: Placed = HashMap::new();
    let widgets = pdf.map(widget_boxes).unwrap_or_default();
    // Glyphs of the font Symbol read through its encoding, over all pages.
    let mut symbols: u64 = 0;
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
            halted = true;
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
            if let Some(note) = found_by.remove(&n) {
                // The separators drawn as marks that were written, said at the top of the page.
                found_by_note(
                    &mut blocks,
                    note,
                    &format!("which separators on page {n} are drawn marks"),
                    n,
                    budget,
                    out,
                );
            }
            let roles = |mcid: u32| pdf.and_then(|pdf| pdf.role_of(n, mcid));
            let cx = Context {
                style: &style,
                tree,
                roles: &roles,
                budget,
                tables: &tables,
                in_part: items.in_part,
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
            links.free(items.kept);
            let height = frame_height(info, Some(&items));
            read_pages.insert(n);
            let here = on_page.get(&n).map_or(&[][..], Vec::as_slice);
            let mut points = annotation_points(n, &mut items, annots, here, &mut spots, out);
            // Each picture's block is looked for at the run nearest to its top, after the
            // annotations' points, in the frame of the runs.
            // The work for each small picture while the page is laid out is charged against
            // the kept bytes, and given back after the page.
            // The cap leaves room for the places themselves (their block and words).
            let cap =
                usize::try_from(links.available() / (PICTURE_WORK + PLACE_COST + PLACE_WORDS))
                    .unwrap_or(usize::MAX);
            let seen = render::vision(info, &items, &furniture, &everywhere, cap);
            let mut work = 0;
            if let Some(render::Seen::Small(list, more, unread)) = &seen {
                if *unread > 0 {
                    // Drawings past the most a page reads: not read, not listed.
                    let (what, verb, it) = if *unread == 1 {
                        ("drawing", "looks", "it is")
                    } else {
                        ("drawings", "look", "they are")
                    };
                    out.loss(
                        format!("p{n}"),
                        format!(
                            "page {n} has {unread} more {what} that {verb} like a barcode or QR \
                             code than a page reads (at most {}): {it} not read and not listed",
                            codes::MAX_REGIONS
                        ),
                        LossReason::Limit,
                        Severity::Warning,
                    );
                }
                work = list.len() as u64 * PICTURE_WORK;
                if links.take_bytes(work).is_err() {
                    work = 0;
                }
                if *more > 0 {
                    let hit = rezzer_limits::LimitHit {
                        limit: "kept text",
                        value: (list.len() + more) as u64 * PICTURE_WORK,
                        max: work,
                        unit: "bytes",
                    };
                    pictures.unlist(*more, n, hit.to_string());
                }
            }
            let tops: Vec<Option<(f32, f32)>> = match &seen {
                Some(render::Seen::Page(v)) => {
                    vec![render::picture_top(v, items.turn, info.width, info.height)]
                }
                Some(render::Seen::Small(list, ..)) => list
                    .iter()
                    .map(|p| Some(((p.framed[0] + p.framed[2]) / 2.0, p.framed[3])))
                    .collect(),
                None => Vec::new(),
            };
            let found: Vec<(f32, f32)> = tops.iter().flatten().copied().collect();
            let mut nearest = quote::nearest_run_spans(&items.runs, &found).into_iter();
            // The middles across and the baselines of the page's runs with ink, by the middle:
            // whether text of a picture's width stands over it and under it (see `Spans`).
            let mut run_mids: Vec<(f32, f32)> = if found.is_empty() {
                Vec::new()
            } else {
                items
                    .runs
                    .iter()
                    .filter(|r| r.text.chars().any(|c| !c.is_whitespace()))
                    .map(|r| (r.x + r.w / 2.0, r.y))
                    .filter(|(x, y)| x.is_finite() && y.is_finite())
                    .collect()
            };
            run_mids.sort_by(|a, b| a.0.total_cmp(&b.0));
            let run_spans = Spans::new(&run_mids);
            // For each picture: the run found and its half width, the picture's top, and the
            // index of its point.
            let probes_from = points.len();
            let near: Vec<Option<Near>> = tops
                .iter()
                .map(|top| {
                    let top = (*top)?;
                    let (at, width) = nearest.next().flatten()?;
                    points.push(at);
                    Some((at, (0.5 * (width.1 - width.0), top.1), points.len() - 1))
                })
                .collect();
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
            // Pictures and form fields go into the frame of the runs, which may be turned.
            let (width, _) = frame_size(info, Some(&items));
            let place = |b: &[f32; 4]| items.turn.place_box(*b, info.width, info.height);
            let images: Vec<[f32; 4]> = if items.turned {
                items.images.iter().map(place).collect()
            } else {
                Vec::new()
            };
            let fields: Vec<[f32; 4]> = if items.turned {
                fields.iter().map(place).collect()
            } else {
                fields
            };
            // The dates of page 1's lines are gathered before the layout takes its runs.
            let mut dates = (n == 1).then(|| layout::page_dates(&items.runs, height));
            let runs = std::mem::take(&mut items.runs);
            // The grids of the page's rules, found once in the frame of its runs for every
            // detector of the layout.
            let structure = crate::regions::structure_in_frame(&items.rules, &items.boxes);
            let drawn = Drawn {
                rules: &items.rules,
                structure: &structure,
                images: if items.turned { &images } else { &items.images },
                widgets: &fields,
                boxes: &items.boxes,
                items: Some(&items),
                lines_unsearched: items.lines_unsearched,
                width,
                body_size: style.body_size,
            };
            symbols += items.symbol;
            let mut laid = layout::page_blocks_at(n, height, runs, &drawn, &cx, &points);
            if let Some(dates) = &mut dates {
                if let Some(more) = laid.dates.take() {
                    dates.merge(more);
                }
                page_date(dates, budget, out);
            }
            if let Some(title) = laid.title.take() {
                match budget.charge_text(title.len()) {
                    Ok(()) => out.found_title = Some(title),
                    Err(hit) => out.loss(
                        "p1",
                        format!("{hit}: the title found on page 1 is not kept"),
                        LossReason::Limit,
                        Severity::Warning,
                    ),
                }
            }
            let annot_probes = &laid.probed[..probes_from.min(laid.probed.len())];
            for (&i, two) in here.iter().zip(annot_probes.chunks(2)) {
                spots[i].anchor = two.iter().flatten().next().cloned();
            }
            if laid.probes_cut && here.iter().any(|&i| spots[i].anchor.is_none()) {
                out.loss(
                    format!("p{n}"),
                    "some annotations stand at the page, not at their block: the page has too \
                     many annotations near its text to place",
                    LossReason::Limit,
                    Severity::Warning,
                );
            }
            // A marked passage whose text was not read in full may be what is missing.
            let partial = items.in_part
                || items.notes.iter().any(|n| matches!(n, Note::Damaged(_)))
                || laid.stop.is_some();
            for &i in here {
                if !annots[i].quads.is_empty()
                    && spots[i].quoted.is_none()
                    && !spots[i].cut
                    && partial
                {
                    unquoted.add(n);
                }
            }
            for (what, why, severity) in laid.notes {
                out.loss(format!("p{n}"), what, why, severity);
            }
            for (at, what, why, severity) in std::mem::take(&mut laid.located) {
                out.loss(at, what, why, severity);
            }
            if let Some(note) = laid.found_by {
                // The footnotes the layout found, said at the top of the page too.
                found_by_note(
                    &mut blocks,
                    note,
                    &format!("which paragraphs on page {n} are footnotes"),
                    n,
                    budget,
                    out,
                );
            }
            let section = out.doc.body.len();
            let first_table = blocks.len();
            let nothing =
                laid.blocks.is_empty() && laid.continued.is_empty() && laid.added.is_empty();
            let (orphans, orphaned) =
                attach(&mut out.doc.body, &placed, laid.continued, laid.added);
            // The layout builds rows only for tables it placed: a tagged table kept as
            // paragraphs stays text on its later pages.
            debug_assert!(
                orphaned.is_none(),
                "rows for a table not kept: {orphaned:?}"
            );
            if let Some(table) = orphaned {
                out.loss(
                    format!("p{n}"),
                    format!(
                        "text page {n} adds to {table} is kept as paragraphs at the end of the \
                         page: the table does not hold its rows"
                    ),
                    LossReason::Damaged,
                    Severity::Warning,
                );
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
            // The page's regions, each at the block it became (its host), rows of a tagged
            // table begun on an earlier page at that table, in the page box.
            let mut page_regions = std::mem::replace(&mut laid.regions, layout::no_regions(n));
            let mut anchors: Vec<Option<Anchor>> =
                laid.blocks.iter().map(layout::block_anchor).collect();
            for &(r, table) in &laid.earlier {
                if let Some((_, _, anchor)) = placed.get(&table).filter(|t| !t.2.is_empty())
                    && let Some(region) = page_regions.regions.get_mut(r)
                {
                    region.host = u32::try_from(anchors.len()).unwrap_or(u32::MAX);
                    anchors.push(Some(Anchor(anchor.clone())));
                }
            }
            // Whether each region holds a run with ink: a figure or a code without a media block
            // of its own keeps the block that holds its text (see below).
            let inked: Vec<bool> = page_regions
                .regions
                .iter()
                .map(|r| {
                    laid.engine_in.as_ref().is_some_and(|p| {
                        r.prims.iter().any(|q| {
                            matches!(*q, layoutregions::PrimId::Text(i)
                                if p.texts.get(i as usize).is_some_and(|t| !t.text.trim().is_empty()))
                        })
                    })
                })
                .collect();
            // The engine report of the page, its regions and hosts set once the media blocks
            // are placed (see below).
            let report = match (regions.is_some(), laid.engine_in.take()) {
                (true, Some(input)) => {
                    let run_blocks = laid
                        .run_blocks
                        .iter()
                        .map(|&b| laid.blocks.get(b as usize).map(block_word))
                        .collect();
                    let run_block_at = laid
                        .run_blocks
                        .iter()
                        .map(|&b| (b as usize) < laid.blocks.len())
                        .zip(&laid.run_blocks)
                        .map(|(ok, &b)| ok.then_some(b as usize))
                        .collect();
                    Some(EnginePage {
                        input,
                        regions: layout::no_regions(n),
                        run_block_at,
                        anchors: Vec::new(),
                        run_blocks,
                        stopped: laid.stop.is_some(),
                        block_words: laid.blocks.iter().map(block_word).collect(),
                        tagged: tree.is_some(),
                    })
                }
                _ => None,
            };
            let laid_blocks = laid.blocks.len();
            // The media block of each picture and code of the page (see `Pictures::list`),
            // and the one media block of a page rendered whole.
            let mut media_hosts: Vec<(layoutregions::PrimId, Anchor)> = Vec::new();
            let mut page_media: Option<Anchor> = None;
            // Where the region engine reads each picture and code of a figure or code region:
            // before the block of the page written after it, and the blocks of the regions
            // read beside it (see `layout::MediaAt`).
            let base = blocks.len();
            let engine_spots: HashMap<layoutregions::PrimId, (usize, std::ops::Range<usize>)> =
                laid.media_at
                    .iter()
                    .map(|m| {
                        let at = |b: usize| base + b.min(laid_blocks);
                        (m.prim, (at(m.at), at(m.near.start)..at(m.near.end)))
                    })
                    .collect();
            // The table cell of each picture that stands in one.
            let cell_spots: CellSpots = laid
                .in_cells
                .iter()
                .filter_map(|&(prim, b, r, c)| {
                    let block = laid.blocks.get(b)?;
                    Some((prim, (base + b, r, c, layout::block_anchor(block))))
                })
                .collect();
            let mut in_cells: Vec<CellPicture> = Vec::new();
            blocks.extend(laid.blocks);
            blocks.extend(orphans);
            // The primitive of each picture listed, by its place in `near`.
            let prims: Vec<Option<layoutregions::PrimId>> = match &seen {
                Some(render::Seen::Small(list, ..)) => list.iter().map(|p| Some(p.prim)).collect(),
                _ => Vec::new(),
            };
            // The width each picture takes in the frame of the runs, by its place in `near`.
            let widths: Vec<(f32, f32)> = match &seen {
                Some(render::Seen::Small(list, ..)) => list
                    .iter()
                    .map(|p| (p.framed[0].min(p.framed[2]), p.framed[0].max(p.framed[2])))
                    .collect(),
                _ => Vec::new(),
            };
            // And its bottom and top.
            let heights: Vec<(f32, f32)> = match &seen {
                Some(render::Seen::Small(list, ..)) => list
                    .iter()
                    .map(|p| (p.framed[1].min(p.framed[3]), p.framed[1].max(p.framed[3])))
                    .collect(),
                _ => Vec::new(),
            };
            let engine_spot = |k: usize| {
                prims
                    .get(k)
                    .copied()
                    .flatten()
                    .and_then(|p| engine_spots.get(&p))
            };
            // Where a picture goes by its run: after the block of the run nearest to its top
            // when the run stands above the top, else before it; at the end without one. A
            // picture of a figure or code region stands where the engine reads it, or at its
            // run among the blocks of the regions read beside it unless text of its width
            // stands over it and under it, or by its run when that run stands over or under it
            // in its column (within its width): that run is its neighbour in the column's
            // reading, wherever the engine's place is.
            let anchored = if near.iter().any(Option::is_some) {
                render::anchor_index(&blocks)
            } else {
                HashMap::new()
            };
            let spots: Vec<usize> = near
                .iter()
                .enumerate()
                .map(|(k, found)| {
                    let by_run = found.map(|(at, (half, top), probe)| {
                        let anchor = laid.probed.get(probe).and_then(Option::as_ref);
                        let after = at.1 > top;
                        let place = render::position(&anchored, blocks.len(), anchor, after);
                        // The run stands over or under the picture, in its column: it overlaps
                        // the picture's width by half the narrower of the two (a run beside the
                        // picture, a label at its height or one that only touches its edge,
                        // is no such neighbour).
                        let column = widths.get(k).is_some_and(|w| {
                            let over = (at.0 + half).min(w.1) - (at.0 - half).max(w.0);
                            over > 0.0 && over >= 0.5 * (2.0 * half).min(w.1 - w.0)
                        });
                        (place, column && anchor.is_some())
                    });
                    // Text in the picture's width over it and under it, at any distance (a
                    // page head line over it and a page number under it count too): then the
                    // engine's place stands unless the run found is of its width (a run beside
                    // it reads in another column). Where one region makes several blocks, this
                    // looks past the neighbour blocks; the engine's place for a picture beside
                    // a label is after the label's region anyway.
                    let column_text = widths.get(k).zip(heights.get(k)).is_some_and(|(w, h)| {
                        let from = run_mids.partition_point(|r| r.0 < w.0);
                        let to = run_mids.partition_point(|r| r.0 <= w.1);
                        run_spans
                            .range(from, to)
                            .is_some_and(|(low, high)| high > h.1 && low < h.0)
                    });
                    match (engine_spot(k), by_run) {
                        (Some(_), Some((p, true))) => p,
                        (Some((_, near)), Some((p, _)))
                            if !column_text && near.start < p && p <= near.end =>
                        {
                            p
                        }
                        (Some((at, _)), _) => *at,
                        (None, p) => p.map_or(blocks.len(), |p| p.0),
                    }
                })
                .collect();
            drop(anchored);
            let mut media: Vec<(usize, Block)> = Vec::new();
            // Pictures too small to be listed on a page that is not rendered whole are
            // counted, at the end of the page.
            if !matches!(seen, Some(render::Seen::Page(_))) {
                let tiny = render::very_small(info, &items);
                if let Some(note) = very_small_note(n, tiny, budget, out) {
                    media.push((blocks.len(), note));
                }
            }
            let here = figures.get(&n).map_or(&[][..], Vec::as_slice);
            // The Figure alt texts no picture took as its caption, with where they go.
            let mut unmatched: Vec<(usize, String)> = Vec::new();
            match seen {
                Some(render::Seen::Page(mut v)) => {
                    // The page's first Figure is the page picture's caption; the others stay
                    // as page text at the picture.
                    let caption = here.first().map(|f| f.alt.clone());
                    let regions = std::mem::take(&mut v.regions);
                    let spot = spots.first().copied().unwrap_or(blocks.len());
                    unmatched.extend(here.iter().skip(1).map(|f| (spot, f.alt.clone())));
                    let charged = budget.add_elements(1).and_then(|()| {
                        budget.charge_text(v.alt.len() + caption.as_ref().map_or(0, String::len))
                    });
                    match charged {
                        Ok(()) => {
                            // Every picture and code of the page is in its one media block.
                            let anchor = Anchor(format!("p{n}:m1"));
                            page_media = Some(anchor.clone());
                            media.push((
                                spots.first().copied().unwrap_or(blocks.len()),
                                Block::Media {
                                    asset: format!("page-{n}"),
                                    alt: None,
                                    anchor: Some(anchor),
                                },
                            ));
                            needs.push(render::Need {
                                page: n,
                                whole: Some(out.doc.assets.len()),
                                crops: Vec::new(),
                                origin: items.origin,
                                cut: items.in_part,
                                fallback: items.fallback,
                                regions,
                                past: v.past,
                                turn: items.turn,
                                size: (info.width, info.height),
                                light: items.images.len() <= render::SCAN_PICTURES,
                                glyphs: items
                                    .codes
                                    .iter()
                                    .filter_map(render::glyph_values)
                                    .collect(),
                            });
                            out.doc.assets.push(render::asset(n, v.alt, caption));
                        }
                        Err(hit) => out.loss(
                            format!("p{n}"),
                            format!("page {n} needs vision but is not listed: {hit}"),
                            LossReason::Limit,
                            Severity::Warning,
                        ),
                    }
                }
                Some(render::Seen::Small(list, ..)) => {
                    let crops = pictures.list(
                        n,
                        (&list, &spots, here),
                        (budget, &links),
                        (&mut media, &mut unmatched, &mut media_hosts),
                        (&cell_spots, &mut in_cells),
                        out,
                    );
                    if !crops.is_empty() {
                        needs.push(render::Need {
                            page: n,
                            whole: None,
                            crops,
                            origin: items.origin,
                            cut: items.in_part,
                            fallback: items.fallback,
                            regions: Vec::new(),
                            past: 0,
                            turn: items.turn,
                            size: (info.width, info.height),
                            light: items.images.len() <= render::SCAN_PICTURES,
                            glyphs: items
                                .codes
                                .iter()
                                .filter_map(render::glyph_values)
                                .collect(),
                        });
                    }
                }
                None => unmatched.extend(here.iter().map(|f| (blocks.len(), f.alt.clone()))),
            }
            // Each picture of a table cell goes into its cell; where the cell is not there (a
            // limit cut the table), it stands as a media block at its place.
            for ((b, r, c, prim), spot, m) in in_cells {
                let table = cell_spots.get(&prim).and_then(|s| s.3.clone());
                let cell = match blocks.get_mut(b) {
                    Some(Block::Table { rows, .. }) => {
                        rows.get_mut(r).and_then(|row| row.get_mut(c))
                    }
                    _ => None,
                };
                match cell {
                    Some(cell) => {
                        // Its figure region stands at the table.
                        if let Some(table) = table {
                            media_hosts.push((prim, table));
                        }
                        cell.media.push(m);
                    }
                    None => {
                        // The next media anchor of the page, as each media block has one.
                        let made = media
                            .iter()
                            .filter(|(_, b)| {
                                matches!(
                                    b,
                                    Block::Media {
                                        anchor: Some(_),
                                        ..
                                    }
                                )
                            })
                            .count();
                        let anchor = Anchor(format!("p{n}:m{}", made + 1));
                        // Its figure region stands at the media block it became.
                        media_hosts.push((prim, anchor.clone()));
                        media.push((
                            spot,
                            Block::Media {
                                asset: m.asset,
                                alt: m.alt,
                                anchor: Some(anchor),
                            },
                        ));
                    }
                }
            }
            if !unmatched.is_empty() {
                // Before the pictures at their place, in their order.
                let texts = figure_texts(n, unmatched, (budget, &links), out);
                media = texts.into_iter().chain(media).collect();
            }
            if !media.is_empty() {
                // One pass: the pictures go between the blocks, and each table of this page
                // moves on by the pictures before it.
                // A stable sort: the Figure texts stay before the pictures at their place.
                media.sort_by_key(|(at, _)| *at);
                let at: Vec<usize> = media.iter().map(|(at, _)| *at).collect();
                for (_, b, _) in placed.values_mut().filter(|t| t.0 == section) {
                    *b += at.partition_point(|&a| a <= *b);
                }
                blocks = render::merge(blocks, media);
            }
            // A figure or a code region stands at the media block of its first picture or code
            // that has one. One whose pictures have no block on the page (page furniture past
            // its first place, a picture not listed, a drawing) keeps the block that holds its
            // text (see `layout::hosts`), and stands at the page when it holds no text with ink:
            // it wrote nothing of its own there.
            let mut media_words = 0usize;
            for (k, region) in page_regions.regions.iter_mut().enumerate() {
                if !matches!(
                    region.kind,
                    layoutregions::Kind::Figure | layoutregions::Kind::Code
                ) {
                    continue;
                }
                let found = page_media.clone().or_else(|| {
                    region.prims.iter().find_map(|p| {
                        media_hosts
                            .iter()
                            .find(|(m, _)| m == p)
                            .map(|(_, a)| a.clone())
                    })
                });
                let media = found.map(|a| {
                    anchors.push(Some(a));
                    media_words += 1;
                    u32::try_from(anchors.len() - 1).unwrap_or(u32::MAX)
                });
                region.host =
                    figure_host(media, inked.get(k).copied().unwrap_or(false), region.host);
            }
            out.doc.regions.extend(rezzer_layout::pdf::pdf_regions(
                std::slice::from_ref(&page_regions),
                &|_, host| anchors.get(host as usize).cloned().flatten(),
                &|_, b| page_box(b, &items, info.width, info.height),
            ));
            if let (Some(all), Some(mut report)) = (regions.as_deref_mut(), report) {
                // The hosts past the page's blocks: the rows of a table begun on an earlier
                // page (a table), then the media blocks.
                let media_from = anchors.len() - media_words;
                debug_assert_eq!(report.block_words.len(), laid_blocks);
                report.block_words.resize(media_from, "table");
                report.block_words.resize(anchors.len(), "media");
                report.anchors = anchors
                    .iter()
                    .map(|a| a.as_ref().map(|a| a.0.clone()))
                    .collect();
                report.regions = page_regions;
                all.push(report);
            }
            links.free(work);
            // What a stop cuts: the page from the first part not built on, or its hidden text.
            let mut stop = laid.stop.map(|hit| {
                let rows_of = |s: usize, b: usize| match out.doc.body.get(s) {
                    Some(Block::Section { blocks, .. }) => match blocks.get(b) {
                        Some(Block::Table { rows, .. }) => rows.len(),
                        _ => 0,
                    },
                    _ => 0,
                };
                let from = (!nothing)
                    .then(|| cut_from(n, laid.cut.unwrap_or(Cut::Block(1)), &placed, &rows_of));
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
        // Annotations of a page whose text was not read stand at the page.
        for &i in on_page.get(&n).map_or(&[][..], Vec::as_slice) {
            if !read_pages.contains(&n) && !annots[i].quads.is_empty() {
                unquoted.add(n);
            }
            spots[i]
                .anchor
                .get_or_insert_with(|| Anchor(format!("p{n}")));
        }
    }
    symbol_loss(symbols, out);
    let places = pictures.finish(out);
    quote::losses(&unquoted, quotes_cut, quote_visits, out);
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
    // A stop in the layout already named the pages from where it stopped.
    if let Some(first) = stopped_at.filter(|_| !halted) {
        let last = pages.last().map_or(first, |p| p.number);
        let why = match budget.check_time() {
            Err(hit) if hit.limit == "cancelled" => "the read was cancelled",
            _ => "the time limit was near, and the pages read before are laid out in the time left",
        };
        out.loss(
            format!("p{first}"),
            format!("text of pages {first} to {last} not read: {why}"),
            LossReason::Limit,
            Severity::Warning,
        );
    }
    if pages.len() < count {
        out.loss(
            "document",
            format!(
                "pages {} to {count} not read: the time limit was reached",
                pages.len() + 1,
            ),
            LossReason::Limit,
            Severity::Warning,
        );
    }
    (links, spots, needs, places)
}

/// What the layout gives the steps after it: the link room, the annotations' places, the
/// pages to render and the small picture assets with their places.
type LaidOut = (
    LinkRoom,
    Vec<annots::Placed>,
    Vec<render::Need>,
    Vec<(usize, render::Places)>,
);

/// Where a picture is placed: the middle of the run found nearest to its top; that run's half
/// width and the height of the picture's top; and the index of the point the layout probes for
/// the run's block.
type Near = ((f32, f32), (f32, f32), usize);

/// The page text for Figure alt texts no picture took as its caption, at their places:
/// "Figure (not matched to a picture): <alt>", so no alt text is lost. What the element budget
/// or the kept bytes no longer allow is said in one loss for the page.
fn figure_texts(
    n: u32,
    texts: Vec<(usize, String)>,
    (budget, room): (&Budget, &LinkRoom),
    out: &mut Out,
) -> Vec<(usize, Block)> {
    let total = texts.len();
    let mut blocks = Vec::with_capacity(total);
    for (k, (at, alt)) in texts.into_iter().enumerate() {
        let words = format!("Figure (not matched to a picture): {alt}");
        let charged = budget
            .add_elements(2)
            .and_then(|()| room.take_bytes(words.len() as u64 + PLACE_COST));
        if let Err(hit) = charged {
            let left = total - k;
            let what = if left == 1 {
                "the alt text of 1 Figure".to_string()
            } else {
                format!("the alt texts of {left} Figures")
            };
            out.loss(
                format!("p{n}"),
                format!("{what} on page {n} that match no picture are not written: {hit}"),
                LossReason::Limit,
                Severity::Warning,
            );
            break;
        }
        blocks.push((at, Block::paragraph(vec![Inline::plain(words)])));
    }
    blocks
}

/// What each small picture of a page takes while the page is laid out, besides what its
/// place keeps: the picture, its top and the run found for it, its point and what the layout
/// keeps for it, its place among the blocks and in reading order, its block, its crop and its
/// entry in the page's set of assets.
const PICTURE_WORK: u64 = (std::mem::size_of::<render::Small>()
    + 2 * std::mem::size_of::<Option<(f32, f32)>>()
    + 2 * std::mem::size_of::<(f32, f32)>()
    + std::mem::size_of::<Option<Near>>()
    + std::mem::size_of::<(f32, f32, usize)>()
    + std::mem::size_of::<Option<usize>>()
    + std::mem::size_of::<u32>()
    + std::mem::size_of::<Option<Anchor>>()
    + 16
    + 2 * std::mem::size_of::<usize>()
    + std::mem::size_of::<(usize, Block)>()
    + std::mem::size_of::<render::Crop>()
    + 16) as u64;

/// Things to put among a page's blocks, each with the index of the block it goes before.
type AtPlace<T> = Vec<(usize, T)>;

/// The table cell each picture of a page stands in (see `layout::Laid::in_cells`): the table's
/// place among the page's blocks, the row, the cell and the table's anchor.
type CellSpots = HashMap<layoutregions::PrimId, (usize, usize, usize, Option<Anchor>)>;

/// A picture written in a table cell: the table's place, row and cell, the place a media block
/// of it would take, and the picture.
type CellPicture = (
    (usize, usize, usize, layoutregions::PrimId),
    usize,
    rezzer_model::CellMedia,
);

/// About what the words of one place take: `small picture, box 1234, 1234 to 1234, 1234`.
const PLACE_WORDS: u64 = 64;

/// What one more small picture asset keeps in the model besides its alt text: the asset,
/// its id and its entry in the identity map.
const PICTURE_COST: u64 = 256;

/// The most bytes the alt text of a small picture asset takes: its name, the pages and the
/// boxes of the places it lists (`render::Places::alt`).
const PICTURE_ALT: u64 = 1_200;

/// What one more place of a small picture keeps besides its words: its block in the page.
const PLACE_COST: u64 = std::mem::size_of::<Block>() as u64;

/// Small pictures listed between two questions to the clock.
const PICTURES_PER_CHECK: usize = 4_096;

/// The small pictures of the document by what they are (`PageItems::images_id`) and, for
/// page furniture, its place, so equal pictures share one asset: the index of the asset and
/// its places. Also the small pictures not listed: how many, on which pages, and why the
/// first were not.
#[derive(Default)]
struct Pictures {
    known: HashMap<(u64, Option<usize>), usize>,
    assets: Vec<(usize, render::Places)>,
    unlisted: Option<(usize, u32, u32, String)>,
}

impl Pictures {
    /// Lists the small pictures `list` of page `n`, whose places among the blocks are
    /// `spots`: one block each, into `media` in reading order, and an asset for each picture
    /// not seen before, named `page-<n>-pic-<k>` with `k` its place in reading order on the
    /// page. A new picture drawn in the marked content of one of the page's `figures` gets
    /// that Figure's alt text as its caption; a Figure that marks no picture gives its alt
    /// text to the page's new picture when there is just one. Every other Figure alt text goes
    /// to `unmatched`, with its place: the place of a picture it marks, else the page's first
    /// picture. Returns the pictures to cut out of the page's render: the first place on the
    /// page of each asset it shows, so a picture whose first page is not rendered is cut from
    /// a later one. What the element budget, the kept bytes or the time no longer allow is
    /// counted for one loss.
    fn list(
        &mut self,
        n: u32,
        (list, spots, figures): (&[render::Small], &[usize], &[render::Figure]),
        (budget, room): (&Budget, &LinkRoom),
        (media, unmatched, hosts): (
            &mut AtPlace<Block>,
            &mut AtPlace<String>,
            &mut Vec<(layoutregions::PrimId, Anchor)>,
        ),
        (cells, in_cells): (&CellSpots, &mut Vec<CellPicture>),
        out: &mut Out,
    ) -> Vec<render::Crop> {
        // The media blocks made so far on the page: each takes the next anchor.
        let mut made = hosts.len();
        let mut order: Vec<usize> = (0..list.len()).collect();
        // Reading order: by place among the blocks, then from the top, then from the left.
        order.sort_by(|&a, &b| {
            let (pa, pb) = (&list[a].framed, &list[b].framed);
            spots[a]
                .cmp(&spots[b])
                .then(pb[3].total_cmp(&pa[3]))
                .then(pa[0].total_cmp(&pb[0]))
        });
        let mut by_mcid: HashMap<u32, usize> = HashMap::new();
        for (f, figure) in figures.iter().enumerate() {
            for &m in &figure.mcids {
                by_mcid.entry(m).or_insert(f);
            }
        }
        // For each Figure: taken as a caption, and the place of a picture it marks.
        let mut taken = vec![false; figures.len()];
        let mut marks: Vec<Option<usize>> = vec![None; figures.len()];
        let mut crops: Vec<render::Crop> = Vec::new();
        let mut on_page: HashSet<usize> = HashSet::new();
        let mut new: Vec<usize> = Vec::new();
        // Pictures and regions of paths that may be codes are counted apart.
        let (mut pics, mut codes) = (0, 0);
        for (k, &i) in order.iter().enumerate() {
            let p = &list[i];
            let words = render::place_alt(p);
            // Page furniture is an asset of its own for each of its places, apart from places
            // of the same picture in the text.
            let key = (p.id, p.furniture.map(|f| f.spot));
            let furniture = p.furniture.is_some();
            let known = self.known.get(&key).copied();
            let checked = if k % PICTURES_PER_CHECK == PICTURES_PER_CHECK - 1 {
                budget.check_time()
            } else {
                Ok(())
            };
            let charged = checked
                .and_then(|()| budget.add_elements(1))
                .and_then(|()| {
                    let fresh = if known.is_none() {
                        PICTURE_COST + PICTURE_ALT
                    } else {
                        0
                    };
                    room.take_bytes(words.len() as u64 + PLACE_COST + fresh)
                });
            if let Err(hit) = charged {
                self.unlist(order.len() - k, n, hit.to_string());
                break;
            }
            // Page furniture takes no Figure: it stands apart from the page's content.
            let figure = p
                .mcid
                .filter(|_| !furniture)
                .and_then(|m| by_mcid.get(&m))
                .copied();
            // Regions of paths are named as codes, pictures as pictures. Page furniture is
            // named at its first place only: its later places take no number.
            let name = if p.vector {
                codes += 1;
                format!("page-{n}-code-{codes}")
            } else if furniture && known.is_some() {
                String::new()
            } else {
                pics += 1;
                format!("page-{n}-pic-{pics}")
            };
            let asset = match known {
                // Page furniture is named once, at its first place.
                Some(at) if furniture => {
                    self.assets[at].1.add(n, p);
                    continue;
                }
                Some(at) => {
                    self.assets[at].1.add(n, p);
                    let asset = self.assets[at].0;
                    if let Some(f) = figure {
                        // The same alt text as the asset's caption is said already.
                        if out.doc.assets[asset].caption.as_deref() == Some(&figures[f].alt) {
                            taken[f] = true;
                        } else {
                            marks[f].get_or_insert(spots[i]);
                        }
                    }
                    asset
                }
                None => {
                    let asset = out.doc.assets.len();
                    let caption = figure.map(|f| {
                        taken[f] = true;
                        figures[f].alt.clone()
                    });
                    out.doc.assets.push(render::picture_asset(
                        name,
                        words.clone(),
                        caption,
                        !p.logo,
                    ));
                    self.known.insert(key, self.assets.len());
                    self.assets.push((asset, render::Places::new(n, p)));
                    if !p.vector && !furniture {
                        new.push(asset);
                    }
                    asset
                }
            };
            if on_page.insert(asset) {
                crops.push(render::Crop {
                    asset,
                    b: p.b,
                    how: p.how,
                });
            }
            // A picture in a table cell is written in the cell, and its region stands at the
            // table.
            if let Some((b, r, c, _)) = cells.get(&p.prim) {
                in_cells.push((
                    (*b, *r, *c, p.prim),
                    spots[i],
                    rezzer_model::CellMedia {
                        asset: out.doc.assets[asset].id.clone(),
                        alt: Some(words),
                    },
                ));
                continue;
            }
            // Each media block of the page has an anchor of its own, `p<N>:m<K>` in reading
            // order, where the region of its picture or code stands.
            made += 1;
            let anchor = Anchor(format!("p{n}:m{made}"));
            hosts.push((p.prim, anchor.clone()));
            media.push((
                spots[i],
                Block::Media {
                    asset: out.doc.assets[asset].id.clone(),
                    alt: Some(words),
                    anchor: Some(anchor),
                },
            ));
        }
        // A Figure that marks no picture: the page's only new picture takes the first one.
        if let [only] = new.as_slice()
            && out.doc.assets[*only].caption.is_none()
            && let Some(f) = (0..figures.len()).find(|&f| !taken[f] && marks[f].is_none())
        {
            out.doc.assets[*only].caption = Some(figures[f].alt.clone());
            taken[f] = true;
        }
        let first = order
            .iter()
            .find(|&&i| list[i].furniture.is_none())
            .map(|&i| spots[i])
            // Only page furniture: the end of the page, as on a page without pictures.
            .unwrap_or(usize::MAX);
        for (f, figure) in figures.iter().enumerate().filter(|(f, _)| !taken[*f]) {
            unmatched.push((marks[f].unwrap_or(first), figure.alt.clone()));
        }
        crops
    }

    /// Counts `count` small pictures of page `n` as not listed, for `why`.
    fn unlist(&mut self, count: usize, n: u32, why: String) {
        self.unlisted = Some(match self.unlisted.take() {
            None => (count, n, n, why),
            Some((c, first, _, w)) => (c + count, first, n, w),
        });
    }

    /// Says in one loss how many small pictures were not listed, and gives the assets with
    /// their places: their alt texts are written once they are read for codes
    /// (`render::apply_codes`).
    fn finish(self, out: &mut Out) -> Vec<(usize, render::Places)> {
        if let Some((count, first, last, why)) = self.unlisted {
            let what = if count == 1 {
                "1 small picture".to_string()
            } else {
                format!("{count} small pictures")
            };
            let pages = if first == last {
                format!("page {first}")
            } else {
                format!("pages {first} to {last}")
            };
            out.loss(
                format!("p{first}"),
                format!("{what} on {pages} not listed: {why}"),
                LossReason::Limit,
                Severity::Warning,
            );
        }
        self.assets
    }
}

/// The points whose blocks the annotations of page `n` stand at, two for each of `here`: the
/// middle of the first quoted glyph (not a number when nothing is quoted), and the middle of
/// the run nearest to the top of the rectangle, for when the first lies in no block. Takes
/// the page's quotes into `placed`. On a page read in a turned frame the points are not
/// in the frame of the runs: the annotations stand at the page, with one Info loss.
fn annotation_points(
    n: u32,
    items: &mut PageItems,
    annots: &[Annot],
    here: &[usize],
    placed: &mut [annots::Placed],
    out: &mut Out,
) -> Vec<(f32, f32)> {
    let mut quotes: HashMap<usize, quote::Quote> = std::mem::take(&mut items.quotes)
        .into_iter()
        .map(|q| (q.annot, q))
        .collect();
    let mut at: Vec<Option<(f32, f32)>> = Vec::with_capacity(here.len());
    for &i in here {
        match quotes.remove(&i) {
            Some(q) => {
                placed[i].quoted = q.text;
                placed[i].cut = q.cut;
                at.push(q.at);
            }
            None => at.push(None),
        }
    }
    if items.turned {
        if !here.is_empty() {
            let k = here.len();
            out.loss(
                format!("p{n}"),
                format!(
                    "{k} {} on page {n} at the page, not at a block: the page is read in a \
                     turned frame",
                    if k == 1 {
                        "annotation stands"
                    } else {
                        "annotations stand"
                    },
                ),
                LossReason::Unsupported,
                Severity::Info,
            );
        }
        return Vec::new();
    }
    let (ox, oy) = items.origin;
    let tops: Vec<(f32, f32)> = here
        .iter()
        .map(|&i| {
            let [x0, y0, x1, y1] = annots[i].rect;
            ((x0 + x1) / 2.0 - ox, y0.max(y1) - oy)
        })
        .collect();
    let none = (f32::NAN, f32::NAN);
    at.into_iter()
        .zip(quote::nearest_runs(&items.runs, &tops))
        .flat_map(|(quoted, near)| [quoted.unwrap_or(none), near.unwrap_or(none)])
        .collect()
}

/// Where each tagged table stands: its section and block in the body, and its anchor.
type Placed = HashMap<usize, (usize, usize, String)>;

/// The rows of the tagged table `table` in `body`, if it is placed.
fn placed_rows<'a>(
    body: &'a mut [Block],
    placed: &Placed,
    table: usize,
) -> Option<&'a mut Vec<Vec<Cell>>> {
    let (s, b, _) = placed.get(&table)?;
    match body.get_mut(*s)? {
        Block::Section { blocks, .. } => match blocks.get_mut(*b)? {
            Block::Table { rows, .. } => Some(rows),
            _ => None,
        },
        _ => None,
    }
}

/// Adds a page's rows and cell text for tagged tables begun on earlier pages to their tables.
/// What has no table or cell to go to is never dropped: it comes back as paragraphs (a row's
/// cells joined by a space), with the name of the first table it was for.
fn attach(
    body: &mut [Block],
    placed: &Placed,
    continued: Vec<(usize, Vec<Vec<Cell>>)>,
    added: Vec<(usize, usize, usize, Vec<Inline>)>,
) -> (Vec<Block>, Option<String>) {
    let mut orphans = Vec::new();
    let mut orphaned: Option<String> = None;
    let mut orphan = |table: usize, parts: Vec<Vec<Inline>>, orphans: &mut Vec<Block>| {
        orphaned.get_or_insert_with(|| match placed.get(&table) {
            Some((_, _, anchor)) => format!("table {anchor}"),
            None => "a tagged table begun on an earlier page".to_string(),
        });
        let mut inlines: Vec<Inline> = Vec::new();
        for part in parts
            .into_iter()
            .filter(|p| !rezzer_model::plain_text(p).trim().is_empty())
        {
            if !inlines.is_empty() {
                inlines.push(Inline::plain(" "));
            }
            inlines.extend(part);
        }
        if !inlines.is_empty() {
            orphans.push(Block::paragraph(inlines));
        }
    };
    for (table, rows) in continued {
        match placed_rows(body, placed, table) {
            Some(kept) => kept.extend(rows),
            None => {
                for row in rows {
                    let parts = row.into_iter().map(|c| c.inlines).collect();
                    orphan(table, parts, &mut orphans);
                }
            }
        }
    }
    for (table, row, cell, inlines) in added {
        let kept = placed_rows(body, placed, table)
            .and_then(|rows| rows.get_mut(row))
            .and_then(|r| r.get_mut(cell));
        match kept {
            Some(kept) => {
                if !kept.inlines.is_empty() && !inlines.is_empty() {
                    kept.inlines.push(Inline::plain(" "));
                }
                kept.inlines.extend(inlines);
            }
            None => orphan(table, vec![inlines], &mut orphans),
        }
    }
    (orphans, orphaned)
}

/// What a stop on page `n` cut first, in words that end in "on": "block 4 on", "row 3 of table
/// p2:t1 on". `rows_of` tells how many rows the table at a section and block holds now.
fn cut_from(n: u32, cut: Cut, placed: &Placed, rows_of: &dyn Fn(usize, usize) -> usize) -> String {
    match cut {
        Cut::Block(k) => format!("block {k} on"),
        Cut::Table(k) => format!("table {k} on"),
        Cut::Item { block, item } => format!("item {item} of list p{n}:b{block} on"),
        Cut::Added { table, row } => match placed.get(&table) {
            Some((_, _, anchor)) => {
                format!("the text it adds to row {} of table {anchor} on", row + 1)
            }
            None => "the text it adds to a table begun on an earlier page on".to_string(),
        },
        Cut::Row { table, row } => match table {
            TableRef::Here(k) => format!("row {row} of table p{n}:t{k} on"),
            TableRef::Earlier(t) => match placed.get(&t) {
                Some((s, b, anchor)) => format!("row {} of table {anchor} on", rows_of(*s, *b) + 1),
                None => format!("row {row} of a table begun on an earlier page on"),
            },
        },
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

/// The document date of page 1 (see [`layout::document_date`]), set in the properties, or
/// an `Uncertain` loss that says why there is none.
fn page_date(dates: &layout::Dates, budget: &Budget, out: &mut Out) {
    match layout::document_date(dates) {
        layout::PageDate::None => {}
        layout::PageDate::Found(found) => {
            match budget.charge_text(found.date.len() + found.from.len()) {
                Ok(()) => out.doc.properties.document_date = Some(found),
                Err(hit) => out.loss(
                    "p1",
                    format!("{hit}: the date found on page 1 is not kept"),
                    LossReason::Limit,
                    Severity::Warning,
                ),
            }
        }
        layout::PageDate::Unsure(what) => {
            out.loss("p1", what, LossReason::Uncertain, Severity::Info);
        }
        layout::PageDate::Limit(what) => {
            out.loss("p1", what, LossReason::Limit, Severity::Info);
        }
    }
}

/// The one loss of a file that says how many characters of the font Symbol were read
/// through the Adobe Symbol encoding.
fn symbol_loss(n: u64, out: &mut Out) {
    if n > 0 {
        let (characters, are) = if n == 1 {
            ("character", "is")
        } else {
            ("characters", "are")
        };
        out.loss(
            "document",
            format!(
                "{n} {characters} of the font Symbol {are} read through the Adobe Symbol \
                 encoding: its ToUnicode map gives Latin letters"
            ),
            LossReason::Uncertain,
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

/// The document being built, with whether something was not read, and the asset bytes.
pub(crate) struct Out {
    pub doc: Document,
    pub partial: bool,
    pub blobs: Vec<Blob>,
    /// The bytes rendered pages and extracted attachments may still take.
    pub blob_room: u64,
    /// The title the head of page 1 gives; it is the document's title when the Info
    /// dictionary and XMP give none.
    pub found_title: Option<String>,
}

impl Out {
    fn new(doc: Document) -> Out {
        Out {
            doc,
            partial: false,
            blobs: Vec::new(),
            blob_room: 0,
            found_title: None,
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
        self.loss_info(location, what, why, severity);
    }

    /// Adds a loss that does not make the read partial, whatever its reason: what it names
    /// stays marked in the document (a page that still needs vision), so nothing is hidden.
    /// The reason stays the true one, such as `Limit` for a bound that stopped the work.
    pub fn loss_info(
        &mut self,
        location: impl Into<String>,
        what: impl Into<String>,
        why: LossReason,
        severity: Severity,
    ) {
        self.doc.losses.push(Loss {
            location: location.into(),
            what: what.into(),
            why,
            severity,
        });
    }

    pub fn finish(mut self) -> ReadOutcome {
        // The title found on page 1 stands in for a title the file does not give.
        let p = &mut self.doc.properties;
        if p.title.as_deref().is_none_or(|t| t.trim().is_empty())
            && let Some(title) = self.found_title.take()
        {
            p.title = Some(title);
            p.title_from = Some(layout::TITLE_FROM.to_string());
        }
        ReadOutcome {
            blobs: self.blobs,
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

/// The lowest and the highest baseline of each range of runs sorted by their middle across:
/// a segment tree (two entries for each run, one lookup of a few steps for each picture), so
/// that whether text of a picture's width stands over it and under it is not a walk over the
/// runs in its width.
struct Spans {
    /// The runs `n..2n` are the leaves; entry `i` below `n` joins its two children `2i` and
    /// `2i + 1`. Entry 0 is unused.
    tree: Vec<(f32, f32)>,
}

impl Spans {
    fn new(runs: &[(f32, f32)]) -> Spans {
        let n = runs.len();
        let mut tree = vec![(0.0, 0.0); 2 * n];
        for (i, r) in runs.iter().enumerate() {
            tree[n + i] = (r.1, r.1);
        }
        for i in (1..n).rev() {
            tree[i] = join(tree[2 * i], tree[2 * i + 1]);
        }
        Spans { tree }
    }

    /// The lowest and the highest baseline of the runs `from..to`; none for an empty range or
    /// one past the runs.
    fn range(&self, from: usize, to: usize) -> Option<(f32, f32)> {
        let n = self.tree.len() / 2;
        if to <= from || to > n {
            return None;
        }
        let (mut lo, mut hi) = (from + n, to + n);
        let mut out: Option<(f32, f32)> = None;
        let mut take = |e: (f32, f32)| out = Some(out.map_or(e, |o| join(o, e)));
        while lo < hi {
            if lo & 1 == 1 {
                take(self.tree[lo]);
                lo += 1;
            }
            if hi & 1 == 1 {
                hi -= 1;
                take(self.tree[hi]);
            }
            lo /= 2;
            hi /= 2;
        }
        out
    }
}

/// The lowest and the highest of two ranges of baselines.
fn join(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    (a.0.min(b.0), a.1.max(b.1))
}

/// The host of a figure or a code region: its media block (`media`, the place of its anchor)
/// when it has one; else the block the layout gave it (`host`, the block that holds most of its
/// text) when it holds text with ink (`inked`); else the page (`u32::MAX`): it wrote nothing of
/// its own there.
fn figure_host(media: Option<u32>, inked: bool, host: u32) -> u32 {
    match media {
        Some(m) => m,
        None if inked => host,
        None => u32::MAX,
    }
}

#[cfg(test)]
mod tests {
    /// Task 22 fix rounds 4 and 5: the segment tree gives the lowest and
    /// highest baseline of every range of runs, as a walk over the range does.
    #[test]
    fn spans_give_the_range_of_baselines() {
        for n in [0usize, 1, 2, 3, 7, 8, 9, 33, 64, 100] {
            let runs: Vec<(f32, f32)> = (0..n)
                .map(|i| (i as f32, ((i * 37 + 11) % 23) as f32))
                .collect();
            let spans = super::Spans::new(&runs);
            for from in 0..=n {
                for to in 0..=n {
                    let walked = runs[from.min(to)..to]
                        .iter()
                        .map(|r| r.1)
                        .fold(None, |m, y| {
                            Some(m.map_or((y, y), |(a, b): (f32, f32)| (a.min(y), b.max(y))))
                        });
                    assert_eq!(spans.range(from, to), walked, "{n}: {from}..{to}");
                }
            }
        }
    }

    /// Fix round 2 (G2): a figure or a code region stands at its media block; without one it
    /// keeps the block of its text when it has text with ink, and stands at the page when not.
    #[test]
    fn a_figure_without_media_keeps_the_block_of_its_text() {
        assert_eq!(super::figure_host(Some(7), true, 3), 7);
        assert_eq!(super::figure_host(Some(7), false, 3), 7);
        assert_eq!(super::figure_host(None, true, 3), 3);
        assert_eq!(super::figure_host(None, false, 3), u32::MAX);
    }

    use rezzer_limits::Limits;

    use super::*;
    use crate::fixtures::{Page, pdf, show};

    #[test]
    fn rows_for_a_table_not_placed_are_kept_as_paragraphs() {
        let cell = |t: &str| Cell {
            inlines: vec![Inline::plain(t)],
            row_span: 1,
            col_span: 1,
            anchor: None,
            media: Vec::new(),
        };
        let table = Block::Table {
            caption: None,
            header_rows: 0,
            role: None,
            rows: vec![vec![cell("a")]],
            alignments: Vec::new(),
            anchor: None,
            layout: None,
        };
        let mut body = vec![Block::Section {
            kind: SectionKind::Page,
            title: None,
            anchor: None,
            blocks: vec![table],
        }];
        let placed: Placed = HashMap::from([(1, (0, 0, "p1:t1".to_string()))]);
        // Table 1 is placed: its row and the text for its cell go to it.
        let (orphans, orphaned) = attach(
            &mut body,
            &placed,
            vec![(1, vec![vec![cell("b")]])],
            vec![(1, 0, 0, vec![Inline::plain("c")])],
        );
        assert!(orphans.is_empty() && orphaned.is_none());
        // Table 2 is not, and table 1 has no cell 5: their text comes back as paragraphs.
        let (orphans, orphaned) = attach(
            &mut body,
            &placed,
            vec![(2, vec![vec![cell("d"), cell("e")], vec![cell("")]])],
            vec![(1, 0, 5, vec![Inline::plain("f")])],
        );
        let texts: Vec<String> = orphans
            .iter()
            .map(|b| match b {
                Block::Paragraph { inlines, .. } => rezzer_model::plain_text(inlines),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(texts, ["d e", "f"]);
        assert_eq!(
            orphaned.as_deref(),
            Some("a tagged table begun on an earlier page")
        );
        let (_, orphaned) = attach(
            &mut body,
            &placed,
            Vec::new(),
            vec![(1, 3, 0, vec![Inline::plain("g")])],
        );
        assert_eq!(orphaned.as_deref(), Some("table p1:t1"));
        match &body[0] {
            Block::Section { blocks, .. } => match &blocks[0] {
                Block::Table { rows, .. } => {
                    let kept: Vec<String> = rows
                        .iter()
                        .flatten()
                        .map(|c| rezzer_model::plain_text(&c.inlines))
                        .collect();
                    assert_eq!(kept, ["a c", "b"]);
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn listing_many_small_pictures_asks_the_clock() {
        // 5,000 different small pictures on one page of a read whose time is spent: the
        // listing asks the clock after 4,096 pictures, keeps those it listed and says how
        // many are not listed and why.
        let mut out = Out::new(Document::new(source_info(&fixtures::source(&[]))));
        let budget = Budget::new(Limits::default());
        budget.cancel();
        let list: Vec<render::Small> = (0..5_000u64)
            .map(|k| {
                let x = (k % 100) as f32 * 5.0;
                let b = [x, 100.0, x + 30.0, 130.0];
                render::Small {
                    prim: layoutregions::PrimId::Picture(k as u32),
                    b,
                    framed: b,
                    id: k,
                    how: None,
                    hint: false,
                    vector: false,
                    furniture: None,
                    logo: false,
                    mcid: None,
                }
            })
            .collect();
        let spots = vec![1; list.len()];
        let room = LinkRoom::new(1 << 30);
        let mut pictures = Pictures::default();
        let (mut media, mut unmatched) = (Vec::new(), Vec::new());
        pictures.list(
            1,
            (&list, &spots, &[]),
            (&budget, &room),
            (&mut media, &mut unmatched, &mut Vec::new()),
            (&HashMap::new(), &mut Vec::new()),
            &mut out,
        );
        let _ = pictures.finish(&mut out);
        assert_eq!(media.len(), PICTURES_PER_CHECK - 1);
        let said: Vec<&str> = out.doc.losses.iter().map(|l| l.what.as_str()).collect();
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].starts_with("905 small pictures on page 1 not listed: ")
                && said[0].contains("cancelled"),
            "{said:?}"
        );
    }

    #[test]
    fn failures_are_grouped_by_run_of_pages() {
        let mut out = Out::new(Document::new(source_info(&fixtures::source(&[]))));
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
    fn a_cut_in_the_text_added_to_an_earlier_row_names_that_row() {
        let placed: Placed = HashMap::from([(7, (2, 1, "p3:t1".to_string()))]);
        let rows = |_: usize, _: usize| 12;
        assert_eq!(
            cut_from(5, Cut::Added { table: 7, row: 4 }, &placed, &rows),
            "the text it adds to row 5 of table p3:t1 on"
        );
        assert_eq!(
            cut_from(
                5,
                Cut::Row {
                    table: TableRef::Earlier(7),
                    row: 1
                },
                &placed,
                &rows
            ),
            "row 13 of table p3:t1 on"
        );
    }

    #[test]
    fn shared_code_decoded_once() {
        // One QR code picture drawn on three pages at three places: one asset, read once.
        let m = barcodes::encode::text(
            barcodes::Symbology::QrCode,
            "https://example.org/once",
            &barcodes::encode::Options::default(),
        )
        .expect("encode");
        let (gray, w, h) = m.gray(4, 4, 0);
        let mut b = crate::fixtures::Builder::default();
        let catalog = b.reserve();
        let tree = b.reserve();
        let image = b.stream(
            &format!(
                "/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray \
                 /BitsPerComponent 8"
            ),
            &gray,
        );
        let mut kids = Vec::new();
        for y in [500, 400, 300] {
            let cs = b.stream("", format!("q 60 0 0 60 260 {y} cm /Q Do Q").as_bytes());
            kids.push(b.add(format!(
                "<< /Type /Page /Parent {tree} 0 R /MediaBox [0 0 595 842] \
                 /Resources << /XObject << /Q {image} 0 R >> >> /Contents {cs} 0 R >>"
            )));
        }
        let list: Vec<String> = kids.iter().map(|k| format!("{k} 0 R")).collect();
        b.set(
            tree,
            format!("<< /Type /Pages /Kids [{}] /Count 3 >>", list.join(" ")),
        );
        b.set(catalog, format!("<< /Type /Catalog /Pages {tree} 0 R >>"));
        let bytes = b.finish(catalog);
        let src = fixtures::source(&bytes);
        let budget = Budget::new(Limits::default());
        let before = render::SCAN_RUNS.with(std::cell::Cell::get);
        let out = read_pdf_in(
            &PdfReader,
            &src,
            &ReadOptions::default(),
            &budget,
            &|| false,
            1 << 30,
            None,
        );
        let runs = render::SCAN_RUNS.with(std::cell::Cell::get) - before;
        assert_eq!(runs, 1);
        assert_eq!(out.document.assets.len(), 1);
        assert_eq!(out.document.assets[0].codes.len(), 1);
    }

    #[test]
    fn crops_stop_when_the_blob_room_is_spent() {
        let bytes = crate::fixtures::small_pictures_twin(2_000);
        let src = fixtures::source(&bytes);
        let opts = ReadOptions {
            extract_assets: true,
            ..ReadOptions::default()
        };
        let budget = Budget::new(Limits::default());
        let room = 100_000;
        let out = read_pdf_in(&PdfReader, &src, &opts, &budget, &|| false, room, None);
        let kept: u64 = out.blobs.iter().map(|b| b.bytes.len() as u64).sum();
        assert!(kept <= room, "{kept}");
        assert!(
            !out.blobs.is_empty() && out.blobs.len() < 2_000,
            "{}: {:#?}",
            out.blobs.len(),
            out.document.losses
        );
        let left: Vec<&Loss> = out
            .document
            .losses
            .iter()
            .filter(|l| l.why == LossReason::Limit)
            .collect();
        assert_eq!(left.len(), 1, "{:#?}", out.document.losses);
        let rest = 2_000 - out.blobs.len();
        assert_eq!(left[0].location, "p1");
        assert!(
            left[0].what.starts_with(&format!(
                "{rest} small pictures are not written, from page 1 on: "
            )),
            "{}",
            left[0].what
        );
        assert!(left[0].what.contains("bytes left for extracted pictures"));
        assert_eq!(out.completeness, Completeness::Partial);
    }

    #[test]
    fn time_stop_between_pages_is_partial() {
        let page = Page::text(show("F1", 12.0, 72.0, 700.0, "Some words."));
        let bytes = pdf(&std::iter::repeat_n(page, 70).collect::<Vec<_>>(), "");
        let src = fixtures::source(&bytes);
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
        let out = read_pdf(
            &PdfReader,
            &src,
            &ReadOptions::default(),
            &budget,
            &stop,
            None,
        );
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

    /// A page drawn as a form: a bold title over a rule no grid takes, a grid of two rows and
    /// two columns and, if `boxed`, a box with text in it. Every detector that looks at drawn
    /// lines runs (without a box, the test whether the page is drawn as a form looks at its
    /// grids).
    fn drawn_form_page(boxed: bool) -> String {
        let mut c = show("F2", 12.0, 72.0, 540.0, "Muster GmbH");
        c += &crate::fixtures::hrule(72.0, 300.0, 534.0);
        c += &show(
            "F1",
            10.0,
            72.0,
            515.0,
            "Erika Mustermann, 12345 Musterstadt",
        );
        c += "0.5 w ";
        for y in [480.0, 450.0, 420.0] {
            c += &format!("72 {y} m 312 {y} l S ");
        }
        for x in [72.0, 192.0, 312.0] {
            c += &format!("{x} 420 m {x} 480 l S ");
        }
        for (x, y, t) in [
            (78.0, 460.0, "Menge"),
            (198.0, 460.0, "Betrag"),
            (78.0, 430.0, "12"),
            (198.0, 430.0, "1.234,56"),
        ] {
            c += &show("F1", 10.0, x, y, t);
        }
        if boxed {
            c += &crate::fixtures::boxed(72.0, 300.0, 200.0, 40.0);
        }
        c += &show("F1", 10.0, 80.0, 315.0, "Bitte beachten");
        c += &show("F1", 10.0, 72.0, 250.0, "A paragraph after the box.");
        c
    }

    /// The grids of a page are found once in each pass that reads them: in the text pass with
    /// the boxes, and once more for all the detectors of the layout, in the frame of the runs
    /// (the text pass does not keep its grids until the page is laid out, see
    /// `regions::structure_in_frame`). Before, each detector searched them on its own. A page
    /// read turned is searched the same twice. The count is taken in the engine's grid search
    /// itself, so any path to it counts. `tables::field_frames` keeps a search of its own by
    /// ruling (it runs before the page's structure is found, see there); the page here draws
    /// no separator marks, so it does not run.
    #[test]
    fn grids_are_found_once_per_page() {
        let searches = || {
            layoutregions::segment::drawn::grid_searches()
                .expect("a test build counts grid searches")
        };
        let pages = [
            Page::text(drawn_form_page(true)),
            Page::text(drawn_form_page(false)),
        ];
        let bytes = pdf(&pages, "");
        let before = searches();
        let out = crate::fixtures::read(&bytes);
        assert_eq!(searches() - before, 4);
        let md = crate::fixtures::markdown(&out);
        assert!(md.contains("| Menge"), "{md}");
        assert!(md.contains("1.234,56"), "{md}");
        // The same page turned a quarter: its text reads upright in a turned frame.
        let turned = Page::text(format!("q 0 1 -1 0 595 0 cm\n{}Q\n", drawn_form_page(true)));
        let before = searches();
        let out = crate::fixtures::read(&pdf(&[turned], ""));
        assert_eq!(searches() - before, 2);
        assert!(crate::fixtures::markdown(&out).contains("| Menge"));
    }
}
