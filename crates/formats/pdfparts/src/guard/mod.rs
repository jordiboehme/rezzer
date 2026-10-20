// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stream guard.
//!
//! Every stream is decoded once into counters that keep no data ([`decode`]). Object streams
//! and cross-reference streams are unpacked while the file is parsed, each under the fixed
//! cap [`BACKSTOP`]. [`measure`] then measures every stream the document keeps, and a page
//! that uses a stream over a limit is refused.

mod decode;

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

use lopdf::{Dictionary, Document, Object, ObjectId};

use decode::Cap;
pub(crate) use decode::{Measure, cap_words, filters_with, measure_bytes, stream_filters, unpack};

use crate::pagetree::{self, PageNode, ResourcesKey};
use crate::{Issue, IssueKind, Limits, OpenError};

/// The most an object stream or a cross-reference stream may unpack to. It never depends on
/// the file.
pub(crate) const BACKSTOP: u64 = 64 << 20;

/// How many pages are checked between two calls of the stop callback.
const PAGES_PER_POLL: usize = 256;

/// How many objects are walked between two calls of the stop callback.
const OBJECTS_PER_POLL: usize = 4096;

fn stopped() -> OpenError {
    OpenError::Limit("stopped by the caller".into())
}

/// The guard's verdict on a document.
#[derive(Debug, Default)]
pub(crate) struct Verdict {
    pub refused_pages: Vec<u32>,
    pub refused_objects: HashSet<ObjectId>,
    pub unpacked: u64,
}

/// Measures every stream of the document once, then refuses each page that reaches a stream
/// over a limit. `unpacked` is what the object and cross-reference streams took already.
pub(crate) fn measure(
    doc: &Document,
    pages: &[PageNode],
    unpacked: u64,
    limits: &Limits,
    stop: &dyn Fn() -> bool,
    issues: &mut Vec<Issue>,
) -> Result<Verdict, OpenError> {
    let mut verdict = Verdict {
        unpacked,
        ..Verdict::default()
    };
    let mut bad: HashMap<ObjectId, String> = HashMap::new();
    let mut total_hit = false;
    for (&id, obj) in &doc.objects {
        let Object::Stream(stream) = obj else {
            continue;
        };
        if stop() {
            return Err(stopped());
        }
        if total_hit {
            bad.insert(
                id,
                "was not measured: the document hit its total limit".into(),
            );
            continue;
        }
        let filters = stream_filters(doc, &stream.dict);
        let budget = limits.max_total_bytes.saturating_sub(verdict.unpacked);
        match measure_bytes(&stream.content, &filters, limits, budget, stop) {
            Measure::Fits(n) => verdict.unpacked += n,
            Measure::Over(cap) => {
                if cap == Cap::Total {
                    total_hit = true;
                    issues.push(Issue::new(
                        "document",
                        IssueKind::Limit,
                        format!(
                            "the streams unpack to more than {} bytes in total",
                            limits.max_total_bytes
                        ),
                    ));
                }
                bad.insert(id, cap_words(cap, limits));
            }
            Measure::Stopped => return Err(stopped()),
        }
    }
    if bad.is_empty() {
        return Ok(verdict);
    }
    let (refused, used) = refuse(doc, pages, &bad, limits, stop, issues)?;
    verdict.refused_pages = refused;
    let mut rest: Vec<_> = bad.iter().filter(|(id, _)| !used.contains(id)).collect();
    rest.sort_by_key(|(id, _)| **id);
    for (id, why) in rest {
        issues.push(Issue::new(
            &format!("object {} {}", id.0, id.1),
            IssueKind::Limit,
            format!("the stream is not read: it {why}"),
        ));
    }
    verdict.refused_objects = bad.into_keys().collect();
    Ok(verdict)
}

/// Collects the references inside `obj`, skipping `/Parent` and `/P` (links up to the page).
fn refs_in(obj: &Object, depth: usize, limits: &Limits, out: &mut Vec<ObjectId>) {
    if depth > limits.max_depth {
        return;
    }
    match obj {
        Object::Reference(id) => out.push(*id),
        Object::Array(a) => a.iter().for_each(|o| refs_in(o, depth + 1, limits, out)),
        Object::Dictionary(d) => dict_refs(d, depth, limits, out),
        Object::Stream(s) => dict_refs(&s.dict, depth, limits, out),
        _ => {}
    }
}

fn dict_refs(d: &Dictionary, depth: usize, limits: &Limits, out: &mut Vec<ObjectId>) {
    for (k, v) in d.iter() {
        if !matches!(k.as_slice(), b"Parent" | b"P") {
            refs_in(v, depth + 1, limits, out);
        }
    }
}

fn is_page_node(doc: &Document, id: ObjectId) -> bool {
    matches!(doc.objects.get(&id), Some(Object::Dictionary(d))
        if d.has_type(b"Page") || d.has_type(b"Pages"))
}

/// Which pages reach a bad stream. Reachability is computed once, backwards from the bad
/// streams over all references, and stops at page nodes, so a link to a refused page does not
/// refuse the page that holds the link. Inherited resources are checked once per node that
/// holds them. Returns the refused page numbers and the bad streams that refused a page.
fn refuse(
    doc: &Document,
    pages: &[PageNode],
    bad: &HashMap<ObjectId, String>,
    limits: &Limits,
    stop: &dyn Fn() -> bool,
    issues: &mut Vec<Issue>,
) -> Result<(Vec<u32>, HashSet<ObjectId>), OpenError> {
    // Every reference is a value the parser charged against the object budget, so the
    // reverse map is bounded by that budget and needs no count of its own.
    let mut back: HashMap<ObjectId, Vec<ObjectId>> = HashMap::new();
    let mut refs = Vec::new();
    for (k, (&src, obj)) in doc.objects.iter().enumerate() {
        if k % OBJECTS_PER_POLL == 0 && stop() {
            return Err(stopped());
        }
        refs.clear();
        refs_in(obj, 0, limits, &mut refs);
        for r in &refs {
            back.entry(*r).or_default().push(src);
        }
    }
    // For each object that reaches a bad stream, the bad stream it reaches.
    let mut origin: HashMap<ObjectId, ObjectId> = bad.keys().map(|k| (*k, *k)).collect();
    let mut todo: Vec<ObjectId> = bad.keys().copied().collect();
    while let Some(t) = todo.pop() {
        if is_page_node(doc, t) {
            continue;
        }
        let o = origin[&t];
        for s in back.get(&t).into_iter().flatten() {
            if !origin.contains_key(s) {
                origin.insert(*s, o);
                todo.push(*s);
            }
        }
    }
    // What each page's resources reach, worked out once per resources object (or per node
    // that holds them as a dictionary).
    let mut inherited: HashMap<ResourcesKey, Option<ObjectId>> = HashMap::new();
    let mut refused = Vec::new();
    let mut used = HashSet::new();
    for (index, node) in pages.iter().enumerate() {
        if index % PAGES_PER_POLL == 0 && stop() {
            return Err(stopped());
        }
        let number = index as u32 + 1;
        let mut hit = origin.get(&node.id).copied();
        if hit.is_none()
            && let Some(key) = pagetree::resources_key(doc, node)
        {
            hit = match inherited.entry(key) {
                Entry::Occupied(e) => *e.get(),
                Entry::Vacant(e) => {
                    refs.clear();
                    if let Some(res) = pagetree::resources(doc, node) {
                        refs_in(res, 0, limits, &mut refs);
                    }
                    *e.insert(refs.iter().find_map(|r| origin.get(r).copied()))
                }
            };
        }
        if let Some(id) = hit {
            used.insert(id);
            issues.push(Issue::new(
                &format!("page {number}"),
                IssueKind::Limit,
                format!(
                    "the page is not read: stream {} {} {}",
                    id.0, id.1, bad[&id]
                ),
            ));
            refused.push(number);
        }
    }
    Ok((refused, used))
}
