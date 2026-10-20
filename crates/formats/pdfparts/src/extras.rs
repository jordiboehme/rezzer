// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Embedded files, optional content layers and article threads.

use std::collections::HashSet;

use lopdf::{Object, ObjectId};

use crate::guard::{Filter, Measure, cap_words, stream_filters, unpack};
use crate::objects::{
    Tree, TreeKey, dict, get, get_date, get_dict, get_name, get_text, reference, resolve,
    text_string, walk_tree,
};
use crate::{Issue, IssueKind, Pdf, PdfDate};

/// A file embedded in the PDF. It is listed and measured, never opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedFile {
    /// The file name: `/UF`, else `/F`, else the key in the name tree.
    pub name: String,
    /// `/Desc` of the file specification.
    pub description: Option<String>,
    /// The media type of the embedded stream (`/Subtype`), for example `image/png`.
    pub mime: Option<String>,
    /// Bytes of the file: `/Params /Size`, else the length the stream guard measured at open.
    /// `None` when neither is known, for example when the stream is over a limit.
    pub size: Option<u64>,
    pub created: Option<PdfDate>,
    pub modified: Option<PdfDate>,
}

/// An optional content group (layer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    pub name: String,
    /// Shown when the file opens, after `/BaseState`, `/ON` and `/OFF` of the default
    /// configuration.
    pub visible: bool,
}

/// An article thread: a chain of beads (areas) that reads as one text across pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    pub title: Option<String>,
    /// The pages the beads lie on, in reading order, each once.
    pub pages: Vec<u32>,
    /// Number of beads read.
    pub beads: u32,
}

/// One embedded file and the stream that holds it.
struct Found {
    file: EmbeddedFile,
    stream: Option<ObjectId>,
}

const THREADS: &str = "article threads";

fn never() -> bool {
    false
}

impl Pdf {
    /// Embedded files from the `/EmbeddedFiles` name tree, then from file attachment
    /// annotations, each stream once.
    pub fn embedded_files(&self) -> Vec<EmbeddedFile> {
        self.found_files().into_iter().map(|f| f.file).collect()
    }

    /// The bytes of embedded file `index` (as listed by [`Pdf::embedded_files`]), unpacked
    /// under the same limits as every stream. A file over a limit is an issue, not data.
    pub fn embedded_file_bytes(&self, index: usize) -> Result<Vec<u8>, Issue> {
        let place = format!("attachment {}", index + 1);
        let fail = |kind, what: &str| Err(Issue::new(&place, kind, what));
        let files = self.found_files();
        let Some(found) = files.get(index) else {
            return fail(IssueKind::Damaged, "there is no such embedded file");
        };
        let Some(id) = found.stream else {
            return fail(IssueKind::Damaged, "the file specification holds no file");
        };
        if self.verdict.refused_objects.contains(&id) {
            return fail(
                IssueKind::Limit,
                "the embedded file is not extracted: its stream is over a limit",
            );
        }
        let Some(Object::Stream(stream)) = self.doc.objects.get(&id) else {
            return fail(IssueKind::Damaged, "the embedded file is not a stream");
        };
        let filters = stream_filters(&self.doc, &stream.dict);
        match unpack(
            &stream.content,
            &filters,
            &self.limits,
            self.limits.max_total_bytes,
            &never,
        ) {
            (Measure::Fits(_), Some(bytes)) => Ok(bytes),
            (Measure::Fits(_), None) => fail(
                IssueKind::Unsupported,
                "the embedded file uses a filter that is not unpacked",
            ),
            (Measure::Over(cap), _) => fail(
                IssueKind::Limit,
                &format!(
                    "the embedded file is not extracted: it {}",
                    cap_words(cap, &self.limits)
                ),
            ),
            (Measure::Stopped, _) => fail(IssueKind::Limit, "stopped by the caller"),
        }
    }

    fn found_files(&self) -> Vec<Found> {
        let doc = &self.doc;
        let mut issues = Vec::new();
        let mut out = Vec::new();
        let mut streams = HashSet::new();
        let tree = self
            .catalog()
            .and_then(|c| get_dict(doc, c, b"Names"))
            .and_then(|n| n.get(b"EmbeddedFiles").ok());
        if let Some(tree) = tree {
            let pairs = walk_tree(
                doc,
                tree,
                Tree::Name,
                &self.limits,
                "attachments",
                &mut issues,
            );
            for (k, spec) in pairs {
                let key = match k {
                    TreeKey::Name(n) => text_string(&n),
                    TreeKey::Number(n) => n.to_string(),
                };
                if let Some(f) = self.found(spec, key, &mut streams) {
                    out.push(f);
                }
            }
        }
        // File attachment annotations, each annotation once.
        let mut seen = HashSet::new();
        let mut visited = 0usize;
        'pages: for node in &self.pages {
            let Some(pd) = doc.objects.get(&node.id).and_then(|o| dict(doc, o)) else {
                continue;
            };
            let Some(Object::Array(annots)) = get(doc, pd, b"Annots") else {
                continue;
            };
            for a in annots {
                visited += 1;
                if visited > self.limits.max_items {
                    issues.push(Issue::new(
                        "attachments",
                        IssueKind::Limit,
                        format!(
                            "more than {} annotations; file attachments may be missing",
                            self.limits.max_items
                        ),
                    ));
                    break 'pages;
                }
                if let Some(r) = reference(a)
                    && !seen.insert(r)
                {
                    continue;
                }
                let Some(ad) = dict(doc, a) else {
                    continue;
                };
                if get_name(doc, ad, b"Subtype") != Some(b"FileAttachment") {
                    continue;
                }
                if let Some(spec) = ad.get(b"FS").ok()
                    && let Some(f) = self.found(spec, String::new(), &mut streams)
                {
                    out.push(f);
                }
            }
        }
        self.note(issues);
        out
    }

    /// One file specification. `None` when its stream is listed already.
    fn found(&self, spec: &Object, key: String, streams: &mut HashSet<ObjectId>) -> Option<Found> {
        let doc = &self.doc;
        let (name, description, ef) = match resolve(doc, spec) {
            Some(Object::String(s, _)) => (text_string(s), None, None),
            Some(Object::Dictionary(d)) => (
                get_text(doc, d, b"UF")
                    .or_else(|| get_text(doc, d, b"F"))
                    .unwrap_or(key),
                get_text(doc, d, b"Desc"),
                get_dict(doc, d, b"EF"),
            ),
            _ => (key, None, None),
        };
        let stream = ef.and_then(|ef| {
            ef.get(b"F")
                .or_else(|_| ef.get(b"UF"))
                .ok()
                .and_then(reference)
        });
        if let Some(id) = stream
            && !streams.insert(id)
        {
            return None;
        }
        let sd = stream.and_then(|id| match self.doc.objects.get(&id) {
            Some(Object::Stream(s)) => Some(s),
            _ => None,
        });
        let params = sd.and_then(|s| get_dict(doc, &s.dict, b"Params"));
        let size = params
            .and_then(|p| match get(doc, p, b"Size") {
                Some(Object::Integer(n)) => u64::try_from(*n).ok(),
                _ => None,
            })
            .or_else(|| self.measured(stream?));
        Some(Found {
            file: EmbeddedFile {
                name,
                description,
                mime: sd
                    .and_then(|s| get_name(doc, &s.dict, b"Subtype"))
                    .map(|m| String::from_utf8_lossy(m).into_owned()),
                size,
                created: params.and_then(|p| get_date(doc, p, b"CreationDate")),
                modified: params.and_then(|p| get_date(doc, p, b"ModDate")),
            },
            stream,
        })
    }

    /// The unpacked length of a stream as the guard measured it at open. `None` when the
    /// guard refused it or a filter stays encoded.
    fn measured(&self, id: ObjectId) -> Option<u64> {
        let Some(Object::Stream(s)) = self.doc.objects.get(&id) else {
            return None;
        };
        if stream_filters(&self.doc, &s.dict).contains(&Filter::Opaque) {
            return None;
        }
        self.verdict.sizes.get(&id).copied()
    }

    /// Optional content groups in the order of `/OCGs`, with their default visibility.
    pub fn layers(&self) -> Vec<Layer> {
        let doc = &self.doc;
        let Some(props) = self
            .catalog()
            .and_then(|c| get_dict(doc, c, b"OCProperties"))
        else {
            return Vec::new();
        };
        let Some(Object::Array(groups)) = get(doc, props, b"OCGs") else {
            return Vec::new();
        };
        let config = get_dict(doc, props, b"D");
        let set = |key: &[u8]| -> HashSet<ObjectId> {
            match config.and_then(|c| get(doc, c, key)) {
                Some(Object::Array(a)) => a.iter().filter_map(reference).collect(),
                _ => HashSet::new(),
            }
        };
        let (on, off) = (set(b"ON"), set(b"OFF"));
        let base_on = config.and_then(|c| get_name(doc, c, b"BaseState")) != Some(b"OFF");
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for g in groups {
            let id = reference(g);
            if let Some(id) = id
                && !seen.insert(id)
            {
                continue;
            }
            let Some(gd) = dict(doc, g) else {
                continue;
            };
            let visible = match id {
                Some(id) if on.contains(&id) => true,
                Some(id) if off.contains(&id) => false,
                _ => base_on,
            };
            out.push(Layer {
                name: get_text(doc, gd, b"Name").unwrap_or_default(),
                visible,
            });
        }
        out
    }

    /// Article threads in the order of the catalog's `/Threads`.
    pub fn threads(&self) -> Vec<Thread> {
        let doc = &self.doc;
        let Some(Object::Array(list)) = self.catalog().and_then(|c| get(doc, c, b"Threads")) else {
            return Vec::new();
        };
        let mut issues = Vec::new();
        let mut out = Vec::new();
        let mut beads_left = self.limits.max_items;
        for (t, thread) in list.iter().enumerate() {
            let Some(td) = dict(doc, thread) else {
                continue;
            };
            let title = get_dict(doc, td, b"I").and_then(|i| get_text(doc, i, b"Title"));
            let mut pages = Vec::new();
            let mut beads = 0u32;
            if let Ok(first) = td.get(b"F") {
                let mut on_page = HashSet::new();
                let first_id = reference(first);
                let mut seen = HashSet::new();
                let mut current = first;
                let broken = loop {
                    if beads_left == 0 {
                        issues.push(Issue::new(
                            THREADS,
                            IssueKind::Limit,
                            format!("more than {} beads", self.limits.max_items),
                        ));
                        break None;
                    }
                    let Some(bead) = dict(doc, current) else {
                        break Some("a bead is not a dictionary");
                    };
                    beads_left -= 1;
                    beads += 1;
                    if let Some(id) = reference(current) {
                        seen.insert(id);
                    }
                    if let Some(p) = bead
                        .get(b"P")
                        .ok()
                        .and_then(reference)
                        .and_then(|p| self.page_number(p))
                        && on_page.insert(p)
                    {
                        pages.push(p);
                    }
                    let Some(next) = bead.get(b"N").ok() else {
                        break Some("the bead chain is broken");
                    };
                    let next_id = reference(next);
                    if next_id.is_some() && next_id == first_id {
                        break None;
                    }
                    if next_id.is_none_or(|n| seen.contains(&n)) {
                        break Some("the bead chain loops back to a bead in the middle");
                    }
                    current = next;
                };
                if let Some(what) = broken {
                    issues.push(Issue::new(
                        THREADS,
                        IssueKind::Damaged,
                        format!("thread {}: {what}", t + 1),
                    ));
                }
            }
            out.push(Thread {
                title,
                pages,
                beads,
            });
        }
        self.note(issues);
        out
    }
}
