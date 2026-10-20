// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! PDF reader: the text of every page in reading order, with headings, lists, tagged tables,
//! running headers and footers, and the text nobody sees on the page kept apart.
//!
//! pdfparts opens the file with its stream guard and gives the object layer: pages, labels
//! and the structure tree. pdf-inspector reads the text with positions from pdfparts'
//! canonical bytes, never from the file itself, and only for pages the guard let through.
//! Scripts and actions are never run, links are never followed.

mod layout;
mod open;
mod text;

#[cfg(test)]
#[path = "../tests/common/mod.rs"]
mod fixtures;

use std::collections::HashMap;

use pdfparts::{IssueKind, PageInfo, Pdf};
use rezzer_limits::Budget;
use rezzer_model::{
    Anchor, Block, CalloutKind, Completeness, Document, FormatId, Inline, Loss, LossReason,
    ReadOptions, ReadOutcome, Reader, ReaderInfo, SectionKind, Severity, Source, StatusColor,
    source_info,
};

use layout::{Context, PageText, Run, Style};
use text::{Extracted, PageItems};

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

/// Reads a file; `text_stop` is asked between two batches of the text parser.
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
    let extracted = extract(&pdf, &pages, &mut out, text_stop);
    lay_out(&pdf, &pages, extracted, budget, &mut out);
    issues(&pdf, &mut out);
    out.finish()
}

/// Reads the text of the pages the guard let through.
fn extract(pdf: &Pdf, pages: &[PageInfo], out: &mut Out, stop: &dyn Fn() -> bool) -> Extracted {
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
    let refused = pdf.refused_pages();
    if !refused.is_empty() {
        // The text parser reads the fonts of every page, also of pages it is not asked for,
        // so a refused stream could reach it through any page.
        out.loss(
            "document",
            format!(
                "text not read on any page: {} refused by the stream guard, and the text \
                 parser reads the fonts of every page",
                page_list(refused)
            ),
            LossReason::Limit,
            Severity::Error,
        );
        return Extracted::default();
    }
    let wanted: Vec<u32> = pages.iter().map(|p| p.number).collect();
    text::pages(pdf.canonical(), &wanted, stop)
}

/// `page 3` or `pages 2, 5 and 9`, at most ten numbers.
fn page_list(pages: &[u32]) -> String {
    let shown: Vec<String> = pages.iter().take(10).map(u32::to_string).collect();
    let more = pages.len().saturating_sub(shown.len());
    match (shown.as_slice(), more) {
        ([one], 0) => format!("page {one} was"),
        (_, 0) => {
            let (last, rest) = shown.split_last().expect("two or more");
            format!("pages {} and {last} were", rest.join(", "))
        }
        _ => format!("pages {} and {more} more were", shown.join(", ")),
    }
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
    let mut halted = false;
    for info in pages {
        let n = info.number;
        let mut blocks = vec![Block::Paragraph {
            inlines: vec![Inline::Status {
                text: marker(info),
                color: StatusColor::Grey,
            }],
            role: Some("page".into()),
            anchor: None,
        }];
        if let (false, Some(items)) = (halted, read.remove(&n)) {
            let roles = |mcid: u32| pdf.role_of(n, mcid);
            let cx = Context {
                style: &style,
                tree,
                roles: &roles,
                budget,
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
            let height = frame_height(info, Some(&items));
            let laid = layout::page_blocks(n, height, items.runs, &cx);
            blocks.extend(laid.blocks);
            let mut stop = laid.stop;
            if stop.is_none() {
                match invisible(n, &items.invisible, &items.hidden_layer, budget, out) {
                    Ok(Some(callout)) => blocks.push(callout),
                    Ok(None) => {}
                    Err(hit) => stop = Some(hit),
                }
            }
            if let Some(hit) = stop {
                out.loss(
                    format!("p{n}"),
                    format!("{hit}: the rest of this page and the later pages are not read"),
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

/// One loss per run of consecutive pages that failed the same way.
fn failures(failed: &[(u32, String, bool)], out: &mut Out) {
    let mut i = 0;
    while i < failed.len() {
        let (first, why, panicked) = &failed[i];
        let mut j = i + 1;
        while j < failed.len()
            && failed[j].0 == failed[j - 1].0 + 1
            && failed[j].1 == *why
            && failed[j].2 == *panicked
        {
            j += 1;
        }
        let last = failed[j - 1].0;
        let pages = if last == *first {
            format!("page {first}")
        } else {
            format!("pages {first} to {last}")
        };
        out.loss(
            format!("p{first}"),
            format!("text of {pages} not read: the text parser failed: {why}"),
            if *panicked {
                LossReason::Internal
            } else {
                LossReason::Damaged
            },
            Severity::Warning,
        );
        i = j;
    }
}

/// The folded callout with the text that puts no glyphs on the page, and its Info loss.
fn invisible(
    page: u32,
    runs: &[Run],
    layer: &[String],
    budget: &Budget,
    out: &mut Out,
) -> Result<Option<Block>, rezzer_limits::LimitHit> {
    if runs.is_empty() && layer.is_empty() {
        return Ok(None);
    }
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
    lines.extend(layer.iter().cloned());
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
        said.push(format!(
            "{} runs of invisible text (render mode 3 or 7)",
            runs.len()
        ));
    }
    if !layer.is_empty() {
        said.push(format!(
            "an invisible text layer (render mode 3) of {} lines",
            layer.len()
        ));
    }
    out.loss(
        format!("p{page}"),
        format!(
            "{}: shown in a folded callout at the end of the page, not in the text",
            said.join(" and ")
        ),
        LossReason::Unsupported,
        Severity::Info,
    );
    Ok(Some(Block::Callout {
        kind: CalloutKind::Details,
        title: Some("Invisible text on this page".into()),
        folded: Some(true),
        blocks,
        anchor: None,
    }))
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
        let bad = |p: u32, why: &str| (p, why.to_string(), false);
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
    fn time_stop_between_batches_is_partial() {
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
        // The time runs out after the first batch: from then on the budget says so too.
        let stop = || {
            asked.set(asked.get() + 1);
            if asked.get() > 1 {
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
