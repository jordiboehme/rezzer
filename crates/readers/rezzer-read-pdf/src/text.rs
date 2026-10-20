// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The text of the pages with positions, read by pdf-inspector. This is the only file that
//! uses pdf-inspector, and it only ever gets the canonical bytes of pdfparts.
//!
//! pdf-inspector parses the whole file on every call, so pages are read in batches of
//! [`BATCH`] with a time check between two batches: one batch is the longest stretch a stop
//! has to wait for.

use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;

use pdf_inspector::extractor::group_into_lines_preserving_all_text;
use pdf_inspector::types::{ItemType, TextLine};
use pdf_inspector::{
    DetectionConfig, PageRotation, PdfOptions, PositionOptions, ScanStrategy, TextItem,
};
use rezzer_limits::contain;

use crate::layout::Run;

/// Pages per pdf-inspector call.
pub(crate) const BATCH: usize = 64;

/// Run and link pairs one batch may test for a URI link over a run.
const LINK_WORK: usize = 16_000_000;

/// A URI link box (x0, y0, x1, y1) and its target.
type Link = ([f32; 4], String);

/// Region that covers any page: PDF pages are at most 14,400 points on a side.
const WHOLE_PAGE: [f32; 4] = [-14_400.0, -14_400.0, 14_400.0, 14_400.0];

/// What pdf-inspector found on one page.
#[derive(Debug, Default)]
pub(crate) struct PageItems {
    pub page: u32,
    /// Visible text in reading order; the runs of one line are next to each other.
    pub runs: Vec<Run>,
    /// Boxes of the pictures the page draws: x0, y0, x1, y1.
    pub images: Vec<[f32; 4]>,
    /// Text that puts no glyphs on the page (render mode 3 or 7), in reading order.
    pub invisible: Vec<Run>,
    /// The lines of an invisible text layer (render mode 3) on a page without visible text,
    /// as an OCR program writes it behind a scan.
    pub hidden_layer: Vec<String>,
    /// The page has more URI links than were matched to its runs.
    pub links_cut: bool,
    /// pdf-inspector turned the page's frame a quarter so its text reads left to right; the
    /// coordinates are not page coordinates then.
    pub turned: bool,
}

/// The pages read, the pages that failed and where a stop cut the reading.
#[derive(Debug, Default)]
pub(crate) struct Extracted {
    pub pages: Vec<PageItems>,
    /// Pages pdf-inspector could not read, with the reason; `true` when it panicked.
    pub failed: Vec<(u32, String, bool)>,
    /// The first page not read because the stop callback said so.
    pub stopped_at: Option<u32>,
}

/// Reads the text of `pages` (1-based, ascending) from the canonical bytes, [`BATCH`] pages
/// per call. `stop` is asked before each call. A batch that fails is read again page by page
/// so one bad page does not cost its neighbours.
pub(crate) fn pages(bytes: &[u8], pages: &[u32], stop: &dyn Fn() -> bool) -> Extracted {
    let mut out = Extracted::default();
    // A failure that a single page repeats is a failure of the file: later batches that fail
    // the same way are not read again page by page.
    let mut whole_file: Option<(String, bool)> = None;
    for batch in pages.chunks(BATCH) {
        if stop() {
            out.stopped_at = batch.first().copied();
            return out;
        }
        let failure = match read_batch(bytes, batch) {
            Ok(found) => {
                out.pages.extend(found);
                continue;
            }
            Err(failure) => failure,
        };
        if whole_file.as_ref() == Some(&failure) {
            out.failed
                .extend(batch.iter().map(|&p| (p, failure.0.clone(), failure.1)));
            continue;
        }
        for (k, &p) in batch.iter().enumerate() {
            if stop() {
                out.stopped_at = Some(p);
                return out;
            }
            match read_batch(bytes, &[p]) {
                Ok(found) => out.pages.extend(found),
                Err(single) if k == 0 && single == failure => {
                    out.failed
                        .extend(batch.iter().map(|&p| (p, failure.0.clone(), failure.1)));
                    whole_file = Some(failure);
                    break;
                }
                Err((why, panicked)) => out.failed.push((p, why, panicked)),
            }
        }
    }
    // An invisible text layer is only given back for pages without visible text.
    let bare: Vec<u32> = out
        .pages
        .iter()
        .filter(|p| p.runs.is_empty() && p.invisible.is_empty())
        .map(|p| p.page)
        .collect();
    let index: HashMap<u32, usize> = out
        .pages
        .iter()
        .enumerate()
        .map(|(i, p)| (p.page, i))
        .collect();
    for batch in bare.chunks(BATCH) {
        if stop() {
            break;
        }
        // A failure here costs only the hidden layer; the page itself was read.
        if let Ok(Ok(found)) = contain(AssertUnwindSafe(|| hidden_layers(bytes, batch))) {
            for (page, lines) in found {
                if let Some(&i) = index.get(&page) {
                    out.pages[i].hidden_layer = lines;
                }
            }
        }
    }
    out
}

/// One pdf-inspector call for `batch`. Every page of the batch gets an entry.
fn read_batch(bytes: &[u8], batch: &[u32]) -> Result<Vec<PageItems>, (String, bool)> {
    let filter: HashSet<u32> = batch.iter().copied().collect();
    let found = contain(AssertUnwindSafe(|| {
        pdf_inspector::extract_text_with_positions_and_rotations_mem_with_options(
            bytes,
            Some(&filter),
            PositionOptions::new(),
        )
    }));
    let (items, turns) = match found {
        Ok(Ok(found)) => found,
        Ok(Err(e)) => return Err((e.to_string(), false)),
        Err(panic) => return Err((panic, true)),
    };
    let mut by_page: Vec<PageItems> = batch
        .iter()
        .map(|&page| PageItems {
            page,
            turned: turns
                .get(&page)
                .is_some_and(|t| *t != PageRotation::Upright),
            ..PageItems::default()
        })
        .collect();
    let slot_of: HashMap<u32, usize> = batch.iter().enumerate().map(|(i, &p)| (p, i)).collect();
    let mut visible = Vec::new();
    // Text that does not read along the x axis (margin notes, stamps) is laid out after the
    // rest of its page, so it does not cut into the lines it crosses.
    let mut turned = Vec::new();
    let mut hidden = Vec::new();
    let mut links: HashMap<u32, Vec<Link>> = HashMap::new();
    for item in items {
        let Some(&slot) = slot_of.get(&item.page) else {
            continue;
        };
        match &item.item_type {
            ItemType::Text if matches!(item.render_mode, Some(3 | 7)) => hidden.push(item),
            ItemType::Text if item.is_horizontal() => visible.push(item),
            ItemType::Text => turned.push(item),
            ItemType::Image => by_page[slot].images.push(rect(&item)),
            ItemType::Link(url) => links
                .entry(item.page)
                .or_default()
                .push((rect(&item), url.clone())),
            // Form field values come from the object layer.
            ItemType::FormField => {}
        }
    }
    let grouped = contain(AssertUnwindSafe(|| {
        (
            group_into_lines_preserving_all_text(visible),
            group_into_lines_preserving_all_text(turned),
            group_into_lines_preserving_all_text(hidden),
        )
    }));
    let (visible, turned, hidden) = grouped.map_err(|panic| (panic, true))?;
    let mut line_no: HashMap<u32, u32> = HashMap::new();
    let mut work = 0usize;
    for (lines, invisible) in [(visible, false), (turned, false), (hidden, true)] {
        for line in lines {
            let Some(&slot) = slot_of.get(&line.page) else {
                continue;
            };
            let n = line_no.entry(line.page).or_insert(0);
            let number = *n;
            *n += 1;
            let page = &mut by_page[slot];
            let on_page = links.get(&line.page).map_or(&[][..], Vec::as_slice);
            // Matching runs to links is bounded per batch; past the bound the page is told.
            let cost = line.items.len().saturating_mul(on_page.len());
            let on_page = if work.saturating_add(cost) > LINK_WORK {
                page.links_cut |= !on_page.is_empty();
                &[][..]
            } else {
                work += cost;
                on_page
            };
            let runs = line_runs(&line, number, on_page);
            if invisible {
                page.invisible.extend(runs);
            } else {
                page.runs.extend(runs);
            }
        }
    }
    Ok(by_page)
}

/// The box of an item: x0, y0, x1, y1.
fn rect(item: &TextItem) -> [f32; 4] {
    [item.x, item.y, item.x + item.width, item.y + item.height]
}

/// The runs of one line. A run that starts a new word begins with a space, decided the way
/// pdf-inspector joins the items of a line.
fn line_runs(line: &TextLine, number: u32, links: &[Link]) -> Vec<Run> {
    let mut runs = Vec::with_capacity(line.items.len());
    for (i, item) in line.items.iter().enumerate() {
        let mut text = item.text.clone();
        if i > 0 && !text.starts_with(char::is_whitespace) && spaced(line, i) {
            text.insert(0, ' ');
        }
        let cx = item.x + item.width / 2.0;
        let cy = item.y + item.height / 2.0;
        let uri = links
            .iter()
            .find(|(r, _)| cx >= r[0] && cx <= r[2] && cy >= r[1] && cy <= r[3])
            .map(|(_, url)| url.clone());
        runs.push(Run {
            page: item.page,
            text,
            x: item.x,
            y: item.y,
            w: item.width,
            h: item.height,
            size: item.font_size,
            bold: item.is_bold,
            italic: item.is_italic,
            mcid: item.mcid.and_then(|m| u32::try_from(m).ok()),
            uri,
            line: number,
        });
    }
    runs
}

/// Whether pdf-inspector puts a space between item `i - 1` and item `i` of a line.
fn spaced(line: &TextLine, i: usize) -> bool {
    let single = |items: Vec<TextItem>| TextLine {
        items,
        y: line.y,
        page: line.page,
        adaptive_threshold: line.adaptive_threshold,
    };
    let first = single(vec![line.items[i - 1].clone()]).text();
    let both = single(vec![line.items[i - 1].clone(), line.items[i].clone()]).text();
    both.get(first.len()..)
        .is_some_and(|rest| rest.starts_with(' '))
}

/// The invisible text layers of pages without visible text, by page. pdf-inspector reads
/// such a layer when it is real text (enough letters and digits, not garbage).
fn hidden_layers(
    bytes: &[u8],
    batch: &[u32],
) -> Result<Vec<(u32, Vec<String>)>, pdf_inspector::PdfError> {
    let regions: Vec<(u32, Vec<[f32; 4]>)> = batch
        .iter()
        .map(|&p| (p.saturating_sub(1), vec![WHOLE_PAGE]))
        .collect();
    let found = pdf_inspector::extract_text_in_regions_mem(bytes, &regions)?;
    Ok(found
        .into_iter()
        .filter_map(|page| {
            let lines: Vec<String> = page
                .regions
                .iter()
                .flat_map(|r| r.text.lines())
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect();
            (!lines.is_empty()).then_some((page.page + 1, lines))
        })
        .collect())
}

/// Pages that need OCR or a look at the picture, with pdf-inspector's reasons. Only `pages`
/// are scanned.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn needs_ocr(bytes: &[u8], pages: &[u32]) -> Result<Vec<(u32, Vec<String>)>, String> {
    let options = PdfOptions::detect_only().detection(DetectionConfig {
        strategy: ScanStrategy::Pages(pages.to_vec()),
        ..DetectionConfig::default()
    });
    let found = contain(AssertUnwindSafe(|| {
        pdf_inspector::process_pdf_mem_with_options(bytes, options)
    }))?
    .map_err(|e| e.to_string())?;
    Ok(found
        .ocr_reasons_by_page
        .into_iter()
        .map(|r| (r.page, r.reasons))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{Page, pdf, show};

    fn canonical(name: &str) -> Vec<u8> {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/corpus/pdf")
            .join(name);
        let bytes = std::fs::read(path).expect("corpus file");
        let pdf = pdfparts::Pdf::open(&bytes, None, &pdfparts::Limits::default(), &|| false)
            .expect("opens");
        pdf.canonical().to_vec()
    }

    #[test]
    fn scanned_page_needs_ocr() {
        let found = needs_ocr(&canonical("scan-truman-letter-nara.pdf"), &[1]).expect("scans");
        assert!(
            found.iter().any(|(p, why)| *p == 1 && !why.is_empty()),
            "{found:?}"
        );
    }

    #[test]
    fn stop_is_asked_between_batches() {
        let page = Page::text(show("F1", 12.0, 72.0, 700.0, "Some words."));
        let bytes = pdf(&vec![page; 70], "");
        let asked = std::cell::Cell::new(0);
        // Yes from the second question on: the first batch is read, the second is not.
        let stop = || {
            asked.set(asked.get() + 1);
            asked.get() > 1
        };
        let all: Vec<u32> = (1..=70).collect();
        let found = pages(&bytes, &all, &stop);
        assert_eq!(found.pages.len(), BATCH);
        assert_eq!(found.stopped_at, Some(65));
        assert_eq!(asked.get(), 2);
    }
}
