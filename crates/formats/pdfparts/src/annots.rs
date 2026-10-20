// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Markup annotations: comments, highlights, notes and drawings with their author and dates.

use std::collections::{HashMap, HashSet};

use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::nav::{page_list, rect};
use crate::objects::{dict, get_date, get_name, get_text, number, reference, resolve};
use crate::{Issue, IssueKind, Pdf, PdfDate};

/// One annotation that is not a link, a popup or a form widget.
#[derive(Debug, Clone, PartialEq)]
pub struct Annot {
    /// The object (`12 0`), or `p<N>#<i>` for a dictionary written straight into the page's
    /// `/Annots`: page N, position i in that array counted from 1.
    pub id: String,
    /// The 1-based page number.
    pub page: u32,
    /// `Highlight`, `Text`, `Ink`, `FreeText` and so on, as written.
    pub subtype: String,
    /// `/T`, by convention the author.
    pub author: Option<String>,
    pub contents: Option<String>,
    pub subject: Option<String>,
    pub created: Option<PdfDate>,
    pub modified: Option<PdfDate>,
    /// `/C` as `#rrggbb`. Gray and CMYK are converted; no colour gives `None`.
    pub color: Option<String>,
    /// The rectangle as written: lower left x and y, upper right x and y.
    pub rect: [f32; 4],
    /// `/QuadPoints` in groups of eight numbers (four corners), in order.
    pub quads: Vec<[f32; 8]>,
    /// The id of the annotation this one replies to (`/IRT`).
    pub in_reply_to: Option<String>,
    /// `/RT` as written: `R` for a reply, `Group` for a part of the parent.
    pub reply_type: Option<String>,
    /// `/State` of a review reply, for example `Accepted` or `Completed`.
    pub state: Option<String>,
}

impl Annot {
    /// What the annotation copies beyond the fixed part: its texts and quads.
    fn weight(&self) -> usize {
        let text = |s: &Option<String>| s.as_ref().map_or(0, String::len);
        let dates = |d: &Option<PdfDate>| d.as_ref().map_or(0, |d| d.raw.len() * 2);
        self.id.len()
            + self.subtype.len()
            + text(&self.author)
            + text(&self.contents)
            + text(&self.subject)
            + dates(&self.created)
            + dates(&self.modified)
            + text(&self.in_reply_to)
            + text(&self.reply_type)
            + text(&self.state)
            + self.quads.len() * 32
    }
}

/// The `#rrggbb` form of a `/C` array: gray, RGB or CMYK components from 0 to 1.
fn color(doc: &Document, obj: Option<&Object>) -> Option<String> {
    let Some(Object::Array(a)) = obj.and_then(|o| resolve(doc, o)) else {
        return None;
    };
    let c: Vec<f32> = a
        .iter()
        .map(|o| resolve(doc, o).and_then(number).map(|v| v.clamp(0.0, 1.0)))
        .collect::<Option<_>>()?;
    let rgb = match c.as_slice() {
        [g] => [*g, *g, *g],
        [r, g, b] => [*r, *g, *b],
        [cy, m, y, k] => [
            (1.0 - cy) * (1.0 - k),
            (1.0 - m) * (1.0 - k),
            (1.0 - y) * (1.0 - k),
        ],
        _ => return None,
    };
    let [r, g, b] = rgb.map(|v| (v * 255.0).round() as u8);
    Some(format!("#{r:02x}{g:02x}{b:02x}"))
}

/// Whole quads of `/QuadPoints`. The second value is true when numbers were left over or
/// were not numbers.
fn quads(doc: &Document, obj: Option<&Object>) -> (Vec<[f32; 8]>, bool) {
    let Some(Object::Array(a)) = obj.and_then(|o| resolve(doc, o)) else {
        return (Vec::new(), obj.is_some());
    };
    let mut out = Vec::with_capacity(a.len() / 8);
    let mut broken = a.len() % 8 != 0;
    for group in a.as_chunks::<8>().0 {
        let mut q = [0.0; 8];
        let mut ok = true;
        for (slot, o) in q.iter_mut().zip(group) {
            match resolve(doc, o).and_then(number) {
                Some(v) => *slot = v,
                None => ok = false,
            }
        }
        if ok {
            out.push(q);
        } else {
            broken = true;
        }
    }
    (out, broken)
}

fn id_text(id: lopdf::ObjectId) -> String {
    format!("{} {}", id.0, id.1)
}

impl Pdf {
    /// Calls `visit` with the page number, the position in the array (from 0) and the entry,
    /// for each `/Annots` entry in page order. An `/Annots` array object that several pages
    /// share is walked once, on the first of them, with one Damaged issue that names the
    /// pages; an entry whose `/P` names one of those pages is given that page. At most
    /// `max_items` entries are visited in all. `visit` ends the walk with an issue.
    pub(crate) fn each_annot<'a>(
        &'a self,
        what: &str,
        issues: &mut Vec<Issue>,
        mut visit: impl FnMut(u32, usize, &'a Object) -> Result<(), Issue>,
    ) {
        let doc = &self.doc;
        // Each page's /Annots entry, and for each array object the pages that list it.
        let mut entries = Vec::with_capacity(self.pages.len());
        let mut sharing: HashMap<ObjectId, Vec<u32>> = HashMap::new();
        for (i, node) in self.pages.iter().enumerate() {
            let page = i as u32 + 1;
            let Some(entry) = doc
                .objects
                .get(&node.id)
                .and_then(|o| dict(doc, o))
                .and_then(|pd| pd.get(b"Annots").ok())
            else {
                continue;
            };
            if let Some(id) = reference(entry) {
                sharing.entry(id).or_default().push(page);
            }
            entries.push((page, entry));
        }
        let mut shared_lists: Vec<&Vec<u32>> = sharing.values().filter(|l| l.len() > 1).collect();
        shared_lists.sort_by_key(|l| l[0]);
        for list in shared_lists {
            let (first, rest) = (list[0], &list[1..]);
            let pages = page_list(rest);
            issues.push(Issue::new(
                "document",
                IssueKind::Damaged,
                if rest.len() == 1 {
                    format!(
                        "page {pages} shares the annotation list of page {first}; its annotations and links are listed on page {first} only"
                    )
                } else {
                    format!(
                        "pages {pages} share the annotation list of page {first}; their annotations and links are listed on page {first} only"
                    )
                },
            ));
        }
        let mut walked = HashSet::new();
        let mut visited = 0usize;
        for (page, entry) in entries {
            let shared = match reference(entry) {
                Some(id) => {
                    if !walked.insert(id) {
                        continue;
                    }
                    sharing.get(&id).filter(|l| l.len() > 1)
                }
                None => None,
            };
            let Some(Object::Array(annots)) = resolve(doc, entry) else {
                continue;
            };
            for (k, a) in annots.iter().enumerate() {
                visited += 1;
                if visited > self.limits.max_items {
                    issues.push(Issue::new(
                        "document",
                        IssueKind::Limit,
                        format!(
                            "the pages list more than {} annotation entries; {what} from page {page} on are not read",
                            self.limits.max_items
                        ),
                    ));
                    return;
                }
                let on = shared
                    .and_then(|pages| {
                        let p = dict(doc, a)?.get(b"P").ok().and_then(reference)?;
                        // The list is in page order.
                        self.page_number(p)
                            .filter(|n| pages.binary_search(n).is_ok())
                    })
                    .unwrap_or(page);
                if let Err(issue) = visit(on, k, a) {
                    issues.push(issue);
                    return;
                }
            }
        }
    }

    /// Every annotation on every page in page order, except links, popups and form widgets.
    /// An annotation that several pages share is listed once, on the first, and so is an
    /// `/Annots` array that pages share.
    pub fn annotations(&self) -> Vec<Annot> {
        let doc = &self.doc;
        let mut issues = Vec::new();
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut budget = self.text_budget();
        // Pages with QuadPoints that do not form whole quads, and how many annotations each.
        let mut broken: Vec<(u32, usize)> = Vec::new();
        self.each_annot("annotations", &mut issues, |page, k, a| {
            let id = match reference(a) {
                Some(r) => {
                    if !seen.insert(r) {
                        return Ok(());
                    }
                    id_text(r)
                }
                None => format!("p{page}#{}", k + 1),
            };
            let Some(ad) = dict(doc, a) else {
                return Ok(());
            };
            let mut bad = 0usize;
            let Some(annot) = self.annot(ad, id, page, &mut bad) else {
                return Ok(());
            };
            if bad > 0 {
                match broken.last_mut() {
                    Some((p, n)) if *p == page => *n += 1,
                    _ => broken.push((page, 1)),
                }
            }
            if !budget.charge(annot.weight()) {
                return Err(budget.limit_issue("document", "annotations"));
            }
            out.push(annot);
            Ok(())
        });
        for (page, n) in broken {
            issues.push(Issue::new(
                &format!("page {page}"),
                IssueKind::Damaged,
                format!(
                    "{n} {} QuadPoints that do not form whole quads; the broken part is left out",
                    if n == 1 {
                        "annotation has"
                    } else {
                        "annotations have"
                    }
                ),
            ));
        }
        self.note(issues);
        out
    }

    fn annot(
        &self,
        ad: &Dictionary,
        id: String,
        page: u32,
        broken_quads: &mut usize,
    ) -> Option<Annot> {
        let doc = &self.doc;
        let subtype = get_name(doc, ad, b"Subtype")?;
        if matches!(subtype, b"Link" | b"Popup" | b"Widget") {
            return None;
        }
        let (quads, broken) = quads(doc, ad.get(b"QuadPoints").ok());
        if broken {
            *broken_quads += 1;
        }
        Some(Annot {
            id,
            page,
            subtype: String::from_utf8_lossy(subtype).into_owned(),
            author: get_text(doc, ad, b"T"),
            contents: get_text(doc, ad, b"Contents"),
            subject: get_text(doc, ad, b"Subj"),
            created: get_date(doc, ad, b"CreationDate"),
            modified: get_date(doc, ad, b"M"),
            color: color(doc, ad.get(b"C").ok()),
            rect: rect(doc, ad.get(b"Rect").ok()).unwrap_or([0.0; 4]),
            quads,
            in_reply_to: ad.get(b"IRT").ok().and_then(reference).map(id_text),
            reply_type: get_name(doc, ad, b"RT").map(|n| String::from_utf8_lossy(n).into_owned()),
            state: get_text(doc, ad, b"State"),
        })
    }
}
