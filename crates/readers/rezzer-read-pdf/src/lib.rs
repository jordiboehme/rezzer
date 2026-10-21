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
//! revisions; `quote.rs` quotes the text under annotations and `annots.rs` puts them at
//! their passage. `render.rs` lists the pages that need vision and renders them on request;
//! `fallback.rs` reads the text of a file pdfparts cannot open with the same text walk.
//! Scripts and actions are never run, links are never followed.

mod annots;
mod fallback;
mod layout;
mod meta;
mod objects;
mod open;
mod order;
mod quote;
mod render;
mod tables;
mod text;

#[cfg(test)]
#[path = "../tests/common/mod.rs"]
mod fixtures;

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
    let mut out = Out::new(Document::new(info));
    // Rendered pages and extracted attachments share one room.
    out.blob_room = budget.limits().max_memory_bytes / 8;
    let (pdf, mut out) = match open::open(src.bytes(), opts.password.as_deref(), budget, out) {
        Ok(opened) => opened,
        Err(open::Refused::Done(outcome)) => return *outcome,
        Err(open::Refused::Unreadable(e, out)) => {
            return fallback::read_file(src, opts, budget, &e, *out, text_stop);
        }
    };
    let stop = || budget.check_time().is_err();
    let pages = pdf.pages(&stop);
    // The revisions are read before any list that draws on pdfparts' budget; the links once,
    // for the text and for the object layer; the annotations before the text, which quotes
    // what they mark.
    let events = objects::revisions(&pdf, budget, &mut out);
    let links = pdf.links();
    let annots = pdf.annotations();
    let marked = quote::marked(&annots);
    let read = Read { src, opts, budget };
    let extracted = extract(&pdf, &pages, &links, &marked, &read, &mut out, text_stop);
    drop(marked);
    let (room, placed, needs) = lay_out(Some(&pdf), &pages, extracted, &annots, budget, &mut out);
    render::render_pages(&pdf, &pages, &needs, opts, budget, &mut out);
    drop(needs);
    annots::read(&annots, placed, budget, &room, &mut out);
    drop(annots);
    objects::read(&pdf, &pages, &links, events, opts, budget, &room, &mut out);
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
            "text not read on any page: the page tree has a loop, a shared node or an untyped \
             node, so the text parser could number the pages differently",
            LossReason::Damaged,
            Severity::Error,
        );
        // What the pages may keep stays for the object layer.
        return text::pages(pdf, &[], links, marked, caps, stop);
    }
    // pdfparts writes refused streams empty, so the other pages are safe to read; the refused
    // ones keep the guard's loss.
    let refused: HashSet<u32> = pdf.refused_pages().iter().copied().collect();
    let wanted: Vec<u32> = pages
        .iter()
        .map(|p| p.number)
        .filter(|n| !refused.contains(n))
        .collect();
    let mut extracted = text::pages(pdf, &wanted, links, marked, caps, stop);
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

/// The height of the page in the frame of its runs, `None` when the frame was turned.
fn frame_height(page: &PageInfo, items: Option<&PageItems>) -> Option<f32> {
    (!items.is_some_and(|i| i.turned)).then_some(page.height)
}

/// One page section per page, with the laid out text. Pages already read are laid out even
/// when a stop cut the reading: layout is bounded by the element and text budgets. Returns
/// what is left of the bytes the file may keep, for the object layer, and where each of
/// `annots` stands with what it quotes.
fn lay_out(
    pdf: Option<&Pdf>,
    pages: &[PageInfo],
    extracted: Extracted,
    annots: &[Annot],
    budget: &Budget,
    out: &mut Out,
) -> (LinkRoom, Vec<annots::Placed>, Vec<render::Need>) {
    let Extracted {
        pages: read,
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
    // The pages whose text was laid out.
    let mut read_pages: HashSet<u32> = HashSet::new();
    let tree = match pdf {
        Some(pdf) if pdf.tagged() => pdf.struct_tree(),
        _ => None,
    };
    let captions = pdf.map(render::figure_captions).unwrap_or_default();
    let furniture = render::furniture(read.values());
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
            // The picture's block is looked for at the run nearest to its top, after the
            // annotations' points; a turned frame has no points.
            let vision = render::vision(info, &items, &furniture);
            let picture = vision
                .as_ref()
                .and_then(render::picture_top)
                .filter(|_| !items.turned)
                .and_then(|top| quote::nearest_runs(&items.runs, &[top])[0].map(|at| (at, top)));
            if let Some((at, _)) = picture {
                points.push(at);
            }
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
            let runs = std::mem::take(&mut items.runs);
            let laid = layout::page_blocks_at(n, height, runs, &drawn, &cx, &points);
            for (&i, two) in here.iter().zip(laid.probed.chunks(2)) {
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
            let picture_block = picture.and_then(|_| laid.probed.last().cloned().flatten());
            blocks.extend(laid.blocks);
            blocks.extend(orphans);
            if let Some(v) = vision {
                let caption = captions.get(&n).cloned();
                let charged = budget.add_elements(1).and_then(|()| {
                    budget.charge_text(v.alt.len() + caption.as_ref().map_or(0, String::len))
                });
                match charged {
                    Ok(()) => {
                        let media = Block::Media {
                            asset: format!("page-{n}"),
                            anchor: None,
                        };
                        // After the block when its run stands above the picture's top.
                        let after = picture.is_some_and(|(at, top)| at.1 > top.1);
                        let at = render::place(&mut blocks, picture_block.as_ref(), after, media);
                        // The tables of this page after it move one block on.
                        for (s, b, _) in placed.values_mut() {
                            if *s == section && *b >= at {
                                *b += 1;
                            }
                        }
                        needs.push(render::Need {
                            page: n,
                            asset: out.doc.assets.len(),
                            cut: items.in_part,
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
    if let Some(first) = stopped_at {
        let last = pages.last().map_or(first, |p| p.number);
        out.loss(
            format!("p{first}"),
            format!("text of pages {first} to {last} not read: the time limit was reached"),
            LossReason::Limit,
            Severity::Warning,
        );
    }
    let count = pdf.map_or(pages.len(), |pdf| pdf.page_count() as usize);
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
    (links, spots, needs)
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
}

impl Out {
    fn new(doc: Document) -> Out {
        Out {
            doc,
            partial: false,
            blobs: Vec::new(),
            blob_room: 0,
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rezzer_limits::Limits;
    use rezzer_model::{Bytes, Confidence, Detection};

    use super::*;
    use crate::fixtures::{Page, pdf, show};

    #[test]
    fn rows_for_a_table_not_placed_are_kept_as_paragraphs() {
        let cell = |t: &str| Cell {
            inlines: vec![Inline::plain(t)],
            row_span: 1,
            col_span: 1,
            anchor: None,
        };
        let table = Block::Table {
            caption: None,
            header_rows: 0,
            role: None,
            rows: vec![vec![cell("a")]],
            alignments: Vec::new(),
            anchor: None,
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
