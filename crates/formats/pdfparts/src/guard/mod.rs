// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stream guard.
//!
//! Every stream is decoded once into counters that keep no data ([`decode`]). lopdf inflates
//! object streams and cross-reference streams while it loads a file, so before lopdf sees the
//! bytes [`prescan`] finds those streams in the raw file ([`scan`]) and measures them. Each of
//! them may unpack to at most [`BACKSTOP`], the same fixed cap lopdf gets, so a file whose object
//! streams lopdf would refuse fails with a Limit instead of losing objects quietly. Everything
//! else is measured by [`measure`] after the load, on the decrypted objects, and a page that
//! uses a stream over a limit is refused.

mod decode;
mod scan;

use std::collections::{HashMap, HashSet};

use lopdf::{Dictionary, Document, Object, ObjectId};

pub(crate) use decode::stream_filters;
use decode::{Measure, cap_words, measure_bytes};

use crate::{Issue, IssueKind, Limits, OpenError};

/// The most an object stream or a cross-reference stream may unpack to, here and in lopdf.
/// It never depends on the file.
pub(crate) const BACKSTOP: u64 = 64 << 20;

/// What the raw scan measured before lopdf loaded the file.
#[derive(Debug, Default)]
pub(crate) struct Prescan {
    /// Object streams and typed cross-reference streams already charged: object, raw length.
    pub measured: HashMap<ObjectId, u64>,
    pub total: u64,
}

/// Limits for streams lopdf decodes while loading.
fn tight(limits: &Limits) -> Limits {
    Limits {
        max_stream_bytes: limits.max_stream_bytes.min(BACKSTOP),
        ..*limits
    }
}

fn stopped() -> OpenError {
    OpenError::Limit("stopped by the caller".into())
}

/// Finds object streams and cross-reference streams in the raw bytes and measures them. A
/// stream over a limit ends the open. In a file that looks encrypted an object stream holds
/// ciphertext: it still ends the open when it is over a limit, but one that fits is not
/// charged, so [`measure`] measures its plain text later.
pub(crate) fn prescan(
    bytes: &[u8],
    base: usize,
    limits: &Limits,
    stop: &dyn Fn() -> bool,
) -> Result<Prescan, OpenError> {
    let mut out = Prescan::default();
    let encrypted = scan::find(bytes, b"/Encrypt", 0).is_some();
    let mut found = scan::typed_streams(bytes, limits, stop)?;
    found.extend(scan::xref_chain(bytes, base, limits)?);
    let tight = tight(limits);
    let mut starts = HashSet::new();
    for s in found {
        if !starts.insert(s.data.0) {
            continue;
        }
        if stop() {
            return Err(stopped());
        }
        let raw = &bytes[s.data.0..s.data.1];
        let budget = limits.max_total_bytes.saturating_sub(out.total);
        let what = if s.is(b"ObjStm") {
            "object stream"
        } else {
            "cross-reference stream"
        };
        match measure_bytes(raw, &s.filters, &tight, budget, stop) {
            Measure::Fits(_) if s.is(b"ObjStm") && encrypted => {}
            Measure::Fits(n) => {
                out.total += n;
                if s.kind.is_some() {
                    out.measured.insert(s.id, raw.len() as u64);
                }
            }
            Measure::Over(cap) => {
                return Err(OpenError::Limit(format!(
                    "{what} {} {} {}",
                    s.id.0,
                    s.id.1,
                    cap_words(cap, &tight)
                )));
            }
            Measure::Stopped => return Err(stopped()),
        }
    }
    Ok(out)
}

/// The guard's verdict on a loaded document.
#[derive(Debug, Default)]
pub(crate) struct Verdict {
    pub refused_pages: Vec<u32>,
    pub refused_objects: HashSet<ObjectId>,
    pub unpacked: u64,
}

fn is_container(d: &Dictionary) -> bool {
    d.has_type(b"ObjStm") || d.has_type(b"XRef")
}

/// Measures every stream of the loaded document that the raw scan did not charge, then
/// refuses each page that reaches a stream over a limit. `pages` holds each page with the
/// resources it inherits from the page tree.
#[allow(clippy::too_many_arguments)]
pub(crate) fn measure(
    doc: &Document,
    bytes: &[u8],
    base: usize,
    pages: &[(ObjectId, Option<&Object>)],
    pre: &Prescan,
    limits: &Limits,
    stop: &dyn Fn() -> bool,
    issues: &mut Vec<Issue>,
) -> Result<Verdict, OpenError> {
    let mut verdict = Verdict {
        unpacked: pre.total,
        ..Verdict::default()
    };
    let tight = tight(limits);
    let mut bad: HashMap<ObjectId, String> = HashMap::new();
    let mut total_hit = false;
    for (&id, obj) in &doc.objects {
        let Object::Stream(stream) = obj else {
            continue;
        };
        let container = is_container(&stream.dict);
        // Skip what the raw scan charged, but only if it is the same stream: a dead object
        // with the same number must not stand in for a live one.
        if container && pre.measured.get(&id) == Some(&(stream.content.len() as u64)) {
            continue;
        }
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
        let caps = if container { &tight } else { limits };
        match measure_bytes(&stream.content, &filters, caps, budget, stop) {
            Measure::Fits(n) => verdict.unpacked += n,
            // An object stream lopdf could not unpack lost its objects: that is a Limit for
            // the whole file, not for one page.
            Measure::Over(cap) if container => {
                return Err(OpenError::Limit(format!(
                    "object stream {} {} {}",
                    id.0,
                    id.1,
                    cap_words(cap, caps)
                )));
            }
            Measure::Over(cap) => {
                if cap == decode::Cap::Total {
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
    missing_members(doc, bytes, base, &tight, stop, issues)?;
    if bad.is_empty() {
        return Ok(verdict);
    }
    let (refused, used) = refuse(doc, pages, &bad, limits, issues);
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

/// lopdf drops an object stream it cannot read without saying so. The cross-reference table
/// still lists its members. If the container is gone, it is read again from the raw file: over
/// a limit is a Limit for the file, anything else is reported as damage.
fn missing_members(
    doc: &Document,
    bytes: &[u8],
    base: usize,
    tight: &Limits,
    stop: &dyn Fn() -> bool,
    issues: &mut Vec<Issue>,
) -> Result<(), OpenError> {
    use lopdf::xref::XrefEntry;
    let mut lost: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    for (&n, entry) in &doc.reference_table.entries {
        if let XrefEntry::Compressed { container, .. } = entry
            && !doc.objects.contains_key(&(n, 0))
        {
            *lost.entry(*container).or_default() += 1;
        }
    }
    for (container, count) in lost {
        if !doc.objects.contains_key(&(container, 0))
            && let Some(XrefEntry::Normal { offset, .. }) = doc.reference_table.get(container)
            && let Some(found) = scan::stream_at(bytes, base + *offset as usize, tight)
        {
            let raw = &bytes[found.data.0..found.data.1];
            match measure_bytes(raw, &found.filters, tight, u64::MAX, stop) {
                Measure::Over(cap) => {
                    return Err(OpenError::Limit(format!(
                        "object stream {container} 0 {}",
                        cap_words(cap, tight)
                    )));
                }
                Measure::Stopped => return Err(stopped()),
                Measure::Fits(_) => {}
            }
        }
        issues.push(Issue::new(
            &format!("object {container} 0"),
            IssueKind::Damaged,
            format!("the object stream could not be read: {count} objects in it are missing"),
        ));
    }
    Ok(())
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
/// refuse the page that holds the link. Returns the refused page numbers and the bad streams
/// that refused at least one page.
fn refuse(
    doc: &Document,
    pages: &[(ObjectId, Option<&Object>)],
    bad: &HashMap<ObjectId, String>,
    limits: &Limits,
    issues: &mut Vec<Issue>,
) -> (Vec<u32>, HashSet<ObjectId>) {
    let all = || (1..=pages.len() as u32).collect::<Vec<u32>>();
    let mut back: HashMap<ObjectId, Vec<ObjectId>> = HashMap::new();
    let mut edges = 0usize;
    let mut refs = Vec::new();
    for (&src, obj) in &doc.objects {
        refs.clear();
        refs_in(obj, 0, limits, &mut refs);
        edges += refs.len();
        if edges > limits.max_items {
            issues.push(Issue::new(
                "document",
                IssueKind::Limit,
                format!(
                    "the file has more than {} references to check; every page is refused",
                    limits.max_items
                ),
            ));
            return (all(), bad.keys().copied().collect());
        }
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
    let mut refused = Vec::new();
    let mut used = HashSet::new();
    for (index, &(page, inherited)) in pages.iter().enumerate() {
        let number = index as u32 + 1;
        let hit = origin.get(&page).copied().or_else(|| {
            refs.clear();
            if let Some(res) = inherited {
                refs_in(res, 0, limits, &mut refs);
            }
            refs.iter().find_map(|r| origin.get(r).copied())
        });
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
    (refused, used)
}
