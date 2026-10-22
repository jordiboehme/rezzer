// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Embedded files, optional content layers and article threads.

use std::collections::HashSet;
use std::sync::Arc;

use lopdf::{Object, ObjectId};

use crate::guard::{Filter, Measure, cap_words, stream_filters, unpack};
use crate::objects::{
    BrokenTexts, Decode, Shared, Spent, TextBudget, Tree, TreeKey, dict, final_id, get, get_date,
    get_dict, get_name, reference, resolve, text_string, walk_tree,
};
use crate::{Issue, IssueKind, Pdf, PdfDate};

/// A file embedded in the PDF. It is listed and measured, never opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedFile {
    /// The file name: `/UF`, else `/F`, else the key in the name tree.
    pub name: Arc<str>,
    /// `/Desc` of the file specification. Files that name one description object share it.
    pub description: Option<Arc<str>>,
    /// The media type of the embedded stream (`/Subtype`), for example `image/png`.
    pub mime: Option<Arc<str>>,
    /// Bytes of the file: `/Params /Size`, else the length the stream guard measured at open.
    /// `None` when neither is known, for example when the stream is over a limit.
    pub size: Option<u64>,
    pub created: Option<PdfDate>,
    pub modified: Option<PdfDate>,
}

/// An optional content group (layer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    /// Layers that name one name object share it.
    pub name: Arc<str>,
    /// Shown when the file opens, after `/BaseState`, `/ON` and `/OFF` of the default
    /// configuration.
    pub visible: bool,
}

/// An article thread: a chain of beads (areas) that reads as one text across pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    /// Threads that name one title or one thread information dictionary share it.
    pub title: Option<Arc<str>>,
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

/// The embedded files as listed, and the stream of each.
#[derive(Debug, Default)]
pub(crate) struct Attachments {
    files: Vec<EmbeddedFile>,
    streams: Vec<Option<ObjectId>>,
}

/// File specifications and streams listed already.
#[derive(Default)]
struct Seen {
    specs: HashSet<ObjectId>,
    streams: HashSet<ObjectId>,
}

impl EmbeddedFile {
    /// The bytes it copies out of the document besides its shared texts: its dates.
    fn weight(&self) -> usize {
        let date = |d: &Option<PdfDate>| d.as_ref().map_or(0, |d| d.raw.len() * 2);
        date(&self.created) + date(&self.modified)
    }
}

const THREADS: &str = "article threads";

fn never() -> bool {
    false
}

impl Pdf {
    /// Embedded files from the `/EmbeddedFiles` name tree, then from file attachment
    /// annotations, each stream once. They are read on the first call and kept.
    pub fn embedded_files(&self) -> &[EmbeddedFile] {
        &self.attachment_list().files
    }

    /// The unpacked length of embedded file `index` as the stream guard measured it at open:
    /// what [`Pdf::embedded_file_bytes`] would return, known before anything is unpacked.
    /// `None` when the file has no stream, the guard refused it or a filter stays encoded.
    pub fn embedded_file_measured(&self, index: usize) -> Option<u64> {
        self.measured((*self.attachment_list().streams.get(index)?)?)
    }

    /// The bytes of embedded file `index` (as listed by [`Pdf::embedded_files`]), unpacked
    /// under the same limits as every stream. A file over a limit is an issue, not data.
    pub fn embedded_file_bytes(&self, index: usize) -> Result<Vec<u8>, Issue> {
        let place = format!("attachment {}", index + 1);
        let fail = |kind, what: &str| Err(Issue::new(&place, kind, what));
        let Some(stream) = self.attachment_list().streams.get(index) else {
            return fail(IssueKind::Damaged, "there is no such embedded file");
        };
        let Some(id) = *stream else {
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

    /// The embedded files, read on the first call and kept.
    fn attachment_list(&self) -> &Attachments {
        self.attachments.get_or_init(|| {
            let (files, streams) = self
                .read_files()
                .into_iter()
                .map(|f| (f.file, f.stream))
                .unzip();
            Attachments { files, streams }
        })
    }

    fn read_files(&self) -> Vec<Found> {
        let doc = &self.doc;
        let mut issues = Vec::new();
        let mut out = Vec::new();
        let mut seen = Seen::default();
        let mut budget = self.text_budget();
        let mut shared = Shared::default();
        let mut broken = BrokenTexts::default();
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
                &mut budget,
                "attachments",
                &mut issues,
            );
            for (k, spec) in pairs {
                let key = match k {
                    TreeKey::Name(n) => text_string(&n),
                    TreeKey::Number(n) => n.to_string(),
                };
                let found = self.found(spec, key, &mut seen, &mut shared, &mut budget);
                broken.after(&mut shared, || format!("attachment {}", out.len() + 1));
                let f = match found {
                    Ok(Some(f)) => f,
                    Ok(None) => continue,
                    Err(Spent) => {
                        issues.push(budget.limit_issue("attachments", "attachments"));
                        issues.extend(broken.issue("attachments"));
                        self.note(issues);
                        return out;
                    }
                };
                if !budget.charge(f.file.weight()) {
                    issues.push(budget.limit_issue("attachments", "attachments"));
                    issues.extend(broken.issue("attachments"));
                    self.note(issues);
                    return out;
                }
                out.push(f);
            }
        }
        // File attachment annotations, each annotation once, however many chains of
        // references name it.
        let mut annots = HashSet::new();
        self.each_annot("attachments", &mut issues, |_, _, a| {
            if let Some(r) = final_id(doc, a)
                && !annots.insert(r)
            {
                return Ok(());
            }
            let Some(ad) = dict(doc, a) else {
                return Ok(());
            };
            if get_name(doc, ad, b"Subtype") != Some(b"FileAttachment") {
                return Ok(());
            }
            let Ok(spec) = ad.get(b"FS") else {
                return Ok(());
            };
            let found = self.found(spec, String::new(), &mut seen, &mut shared, &mut budget);
            broken.after(&mut shared, || format!("attachment {}", out.len() + 1));
            let Ok(found) = found else {
                return Err(budget.limit_issue("attachments", "attachments"));
            };
            if let Some(f) = found {
                if !budget.charge(f.file.weight()) {
                    return Err(budget.limit_issue("attachments", "attachments"));
                }
                out.push(f);
            }
            Ok(())
        });
        issues.extend(broken.issue("attachments"));
        self.note(issues);
        out
    }

    /// One file specification. `None` when it or its stream is listed already.
    fn found(
        &self,
        spec: &Object,
        key: String,
        seen: &mut Seen,
        shared: &mut Shared,
        budget: &mut TextBudget<'_>,
    ) -> Result<Option<Found>, Spent> {
        let doc = &self.doc;
        if let Some(id) = final_id(doc, spec)
            && !seen.specs.insert(id)
        {
            return Ok(None);
        }
        let (name, description, ef) = match resolve(doc, spec) {
            Some(Object::String(..)) => (shared.text(doc, spec, Decode::Text, budget)?, None, None),
            Some(Object::Dictionary(d)) => {
                let name = match shared.text_at(doc, d, b"UF", budget)? {
                    Some(n) => Some(n),
                    None => shared.text_at(doc, d, b"F", budget)?,
                };
                let description = shared.text_at(doc, d, b"Desc", budget)?;
                (name, description, get_dict(doc, d, b"EF"))
            }
            _ => (None, None, None),
        };
        let name = name.unwrap_or_else(|| key.into());
        let stream = ef.and_then(|ef| {
            ef.get(b"F")
                .or_else(|_| ef.get(b"UF"))
                .ok()
                .and_then(|f| final_id(doc, f))
        });
        if let Some(id) = stream
            && !seen.streams.insert(id)
        {
            return Ok(None);
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
        let mime = match sd {
            Some(s) => shared.text_at(doc, &s.dict, b"Subtype", budget)?,
            None => None,
        };
        Ok(Some(Found {
            file: EmbeddedFile {
                name,
                description,
                mime,
                size,
                created: params.and_then(|p| get_date(doc, p, b"CreationDate")),
                modified: params.and_then(|p| get_date(doc, p, b"ModDate")),
            },
            stream,
        }))
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
                Some(Object::Array(a)) => a.iter().filter_map(|o| final_id(doc, o)).collect(),
                _ => HashSet::new(),
            }
        };
        let (on, off) = (set(b"ON"), set(b"OFF"));
        let base_on = config.and_then(|c| get_name(doc, c, b"BaseState")) != Some(b"OFF");
        let mut budget = self.text_budget();
        let mut shared = Shared::default();
        let mut broken = BrokenTexts::default();
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for g in groups {
            // A group named through several chains of references is one layer.
            let id = final_id(doc, g);
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
            let name = shared.text_at(doc, gd, b"Name", &mut budget);
            broken.after(&mut shared, || format!("layer {}", out.len() + 1));
            let Ok(name) = name.map(|n| n.unwrap_or_else(|| "".into())) else {
                self.note(vec![budget.limit_issue("layers", "layers")]);
                break;
            };
            if !budget.charge(0) {
                self.note(vec![budget.limit_issue("layers", "layers")]);
                break;
            }
            out.push(Layer { name, visible });
        }
        self.note(broken.issue("layers").into_iter().collect());
        out
    }

    /// The title of a thread: `/Title` of its `/I` dictionary, read once for each dictionary
    /// that many threads name.
    fn thread_title(
        &self,
        thread: &lopdf::Dictionary,
        shared: &mut Shared,
        budget: &mut TextBudget<'_>,
    ) -> Result<Option<Arc<str>>, Spent> {
        let doc = &self.doc;
        let Ok(info) = thread.get(b"I") else {
            return Ok(None);
        };
        let id = final_id(doc, info);
        if let Some(id) = id
            && let Some(t) = shared.found.get(&id)
        {
            return Ok(t.clone());
        }
        let title = match dict(doc, info) {
            Some(i) => shared.text_at(doc, i, b"Title", budget)?,
            None => None,
        };
        if let Some(id) = id {
            shared.found.insert(id, title.clone());
        }
        Ok(title)
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
        let mut budget = self.text_budget();
        let mut shared = Shared::default();
        let mut broken = BrokenTexts::default();
        for (t, thread) in list.iter().enumerate() {
            let Some(td) = dict(doc, thread) else {
                continue;
            };
            let title = self.thread_title(td, &mut shared, &mut budget);
            broken.after(&mut shared, || format!("thread {}", t + 1));
            let (Ok(title), true) = (title, budget.charge(0)) else {
                issues.push(budget.limit_issue(THREADS, "article threads"));
                break;
            };
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
                    let Some(next_id) = reference(next) else {
                        break Some("a bead's /N is not an indirect object");
                    };
                    if Some(next_id) == first_id {
                        break None;
                    }
                    if seen.contains(&next_id) {
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
        issues.extend(broken.issue(THREADS));
        self.note(issues);
        out
    }
}
