// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The change words of one save. Pages are compared by their object ids, `/Contents`
//! references and stream lengths, never by decoded content; fields by their `/V`. An object
//! a higher word explains (the content stream of a changed page, the array that lists the
//! annotations of a page, the signature dictionary of a signed field) gets no word of its own.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use lopdf::{Dictionary, Object, ObjectId};

use super::{Diff, Halt, Run, Slot, as_dict, id_of, structural, type_word, value_text};
use crate::nav::page_list;
use crate::objects::text_string;
use crate::parse::history::View;
use crate::{Issue, IssueKind};

/// The pages of one revision, in page order.
#[derive(Default)]
pub(crate) struct Pages {
    ids: Vec<ObjectId>,
    /// The number of each page object (the first, for a page listed twice).
    num: HashMap<ObjectId, u32>,
    /// Per page: its content streams with their lengths.
    content: Vec<Vec<(ObjectId, Option<usize>)>>,
    /// Per page: the indirect arrays that list its content streams and its annotations.
    contents_array: Vec<Option<u32>>,
    annots_array: Vec<Option<u32>>,
    /// The page each annotation is listed on.
    annots: HashMap<ObjectId, u32>,
    /// Every object the pages were read from: the catalog, page tree nodes, content arrays,
    /// content streams and their lengths, annotation arrays. A save that changes none of
    /// them leaves the pages as they are.
    deps: HashSet<u32>,
}

/// One object a save lists with a value other than before.
struct Changed {
    n: u32,
    /// The id the save gives the object, when it is in use.
    id: Option<ObjectId>,
    old: Option<Rc<Object>>,
    new: Option<Rc<Object>>,
    /// The save lists the object, but it cannot be read at its offset.
    unreadable: bool,
}

fn name_is(d: &Dictionary, key: &[u8], want: &[u8]) -> bool {
    matches!(d.get(key), Ok(Object::Name(n)) if n == want)
}

fn is_catalog(d: &Dictionary) -> bool {
    name_is(d, b"Type", b"Catalog")
}

fn is_annot(d: &Dictionary) -> bool {
    name_is(d, b"Type", b"Annot") || (d.get(b"Subtype").is_ok() && d.get(b"Rect").is_ok())
}

/// A field, or a field merged with its widget. Markup annotations use `/T` for the author.
fn is_field(d: &Dictionary) -> bool {
    let widget_or_none = match d.get(b"Subtype") {
        Ok(Object::Name(s)) => s == b"Widget",
        _ => true,
    };
    d.get(b"FT").is_ok() || (d.get(b"T").is_ok() && widget_or_none)
}

fn ref_num(o: Option<&Object>) -> Option<u32> {
    match o {
        Some(Object::Reference(id)) => Some(id.0),
        _ => None,
    }
}

impl Run<'_, '_> {
    /// The change words of one save, and the pages after it when they were read (else
    /// `prev`, which is still right).
    pub(super) fn diff(
        &mut self,
        d: &Diff,
        prev: Option<Rc<Pages>>,
    ) -> Result<(Vec<String>, Option<Rc<Pages>>), Halt> {
        let before = d.before();
        let after = d.after();
        let changed = self.changed(d, &before, &after)?;
        // The pages are walked once, and again only after a save that changes an object
        // they were read from.
        let b = match prev {
            Some(p) => p,
            None => Rc::new(self.pages(&before, d.root.0)?),
        };
        let tree_changed =
            ref_num(d.root.0) != ref_num(d.root.1) || changed.iter().any(|c| b.deps.contains(&c.n));
        let a = if tree_changed {
            Rc::new(self.pages(&after, d.root.1)?)
        } else {
            b.clone()
        };
        let pages = Some((b, a));
        let changed_nums: HashSet<u32> = changed.iter().map(|c| c.n).collect();
        let mut explained = HashSet::new();
        let mut words = Vec::new();
        if let Some((b, a)) = &pages
            && !Rc::ptr_eq(b, a)
        {
            words = page_words(b, a, &changed_nums, &mut explained);
            if words
                .iter()
                .any(|w| w.starts_with("added page") || w.starts_with("removed page"))
            {
                for c in &changed {
                    let pages_node = |o: &Option<Rc<Object>>| {
                        o.as_deref()
                            .and_then(as_dict)
                            .is_some_and(|d| name_is(d, b"Type", b"Pages"))
                    };
                    if pages_node(&c.old) || pages_node(&c.new) {
                        explained.insert(c.n);
                    }
                }
            }
        }
        let root_nums = [ref_num(d.root.0), ref_num(d.root.1)];
        let info_nums = [ref_num(d.info.0), ref_num(d.info.1)];
        let mut each: Vec<(u32, Vec<String>)> = Vec::new();
        for c in &changed {
            let new_d = c.new.as_deref().and_then(as_dict);
            let old_d = c.old.as_deref().and_then(as_dict);
            let mut w = Vec::new();
            if c.unreadable {
                w.push(format!("changed object {} (unreadable)", c.n));
            } else if root_nums.contains(&Some(c.n)) || new_d.or(old_d).is_some_and(is_catalog) {
                w.push("changed the catalog".to_string());
            } else if info_nums.contains(&Some(c.n)) {
                w.push("changed document information".to_string());
            } else if let Some(nd) = new_d
                && is_field(nd)
            {
                w = self.field_words(&before, &after, old_d, nd, &mut explained)?;
            }
            if c.old.is_none()
                && !c.unreadable
                && let Some(nd) = new_d
                && is_annot(nd)
            {
                let page = annot_page(nd, c, pages.as_ref().map(|p| p.1.as_ref()));
                if let (Some(page), Some((_, a))) = (page, &pages) {
                    let i = page as usize - 1;
                    explained.insert(a.ids[i].0);
                    explained.extend(a.annots_array[i]);
                }
                if w.is_empty() {
                    let sub = match nd.get(b"Subtype") {
                        Ok(Object::Name(s)) => String::from_utf8_lossy(s).into_owned(),
                        _ => "annotation".to_string(),
                    };
                    w.push(match page {
                        Some(p) => format!("added annotation {sub} on page {p}"),
                        None => format!("added annotation {sub}"),
                    });
                }
            }
            each.push((c.n, w));
        }
        for (c, (_, w)) in changed.iter().zip(each.iter_mut()) {
            if !w.is_empty() || explained.contains(&c.n) {
                continue;
            }
            w.push(match (&c.old, &c.new) {
                (None, Some(new)) => format!("added object {} ({})", c.n, type_word(new)),
                (Some(_), Some(new)) => format!("changed object {} ({})", c.n, type_word(new)),
                _ => format!("changed object {} (free)", c.n),
            });
        }
        words.extend(each.into_iter().flat_map(|(_, w)| w));
        for w in &words {
            self.copy(w.len())?;
        }
        Ok((words, pages.map(|(_, a)| a)))
    }

    /// The objects a save lists whose value differs from before, by number.
    fn changed(&mut self, d: &Diff, before: View, after: View) -> Result<Vec<Changed>, Halt> {
        let mut nums: Vec<u32> = d.save.entries.keys().copied().collect();
        nums.sort_unstable();
        let mut out = Vec::new();
        for n in nums {
            let new_slot = d.save.entries[&n];
            let old_slot = d.state.get(&n).copied();
            if Some(new_slot) == old_slot || (new_slot == Slot::Free && old_slot.is_none()) {
                continue;
            }
            self.visit(1)?;
            let old = match old_slot {
                Some(Slot::Used(e)) => self.load(before, id_of(n, e))?,
                _ => None,
            };
            let id = match new_slot {
                Slot::Used(e) => Some(id_of(n, e)),
                Slot::Free => None,
            };
            let new = match new_slot {
                Slot::Used(e) => self.load(after, id_of(n, e))?,
                Slot::Free => None,
            };
            if new.is_none() && matches!(new_slot, Slot::Used(_)) {
                // Counted by the loader for the issue.
                out.push(Changed {
                    n,
                    id,
                    old,
                    new,
                    unreadable: true,
                });
                continue;
            }
            if old == new
                || old.as_deref().is_some_and(structural)
                || new.as_deref().is_some_and(structural)
            {
                continue;
            }
            out.push(Changed {
                n,
                id,
                old,
                new,
                unreadable: false,
            });
        }
        Ok(out)
    }

    /// The words for a changed or added field.
    fn field_words(
        &mut self,
        before: View,
        after: View,
        old: Option<&Dictionary>,
        new: &Dictionary,
        explained: &mut HashSet<u32>,
    ) -> Result<Vec<String>, Halt> {
        let (name, kind) = self.field_name(after, new)?;
        if kind.as_deref() == Some(b"Sig".as_slice()) {
            let new_v = new.get(b"V").ok();
            let old_v = old.and_then(|o| o.get(b"V").ok());
            let mut w = Vec::new();
            if old.is_none() {
                w.push(format!("added signature field {name}"));
            }
            if new_v.is_some() && new_v != old_v {
                w.push(format!("signed field {name}"));
                explained.extend(ref_num(new_v));
            }
            return Ok(w);
        }
        let Some(old) = old else {
            return Ok(Vec::new());
        };
        let vo = self.resolve(before, old.get(b"V").ok())?;
        let vn = self.resolve(after, new.get(b"V").ok())?;
        if vo == vn {
            return Ok(Vec::new());
        }
        explained.extend(ref_num(new.get(b"V").ok()));
        Ok(vec![format!(
            "field {name} value changed from {} to {}",
            value_text(vo.as_deref()),
            value_text(vn.as_deref())
        )])
    }

    /// The full name of a field (its `/T` and those of its parents) and its `/FT`, inherited.
    fn field_name(
        &mut self,
        view: View,
        d: &Dictionary,
    ) -> Result<(String, Option<Vec<u8>>), Halt> {
        let mut parts = Vec::new();
        let mut kind = None;
        let mut seen = HashSet::new();
        let mut next = name_step(d, &mut parts, &mut kind);
        for _ in 0..self.limits.max_depth {
            let Some(p) = next else {
                break;
            };
            if !seen.insert(p) {
                break;
            }
            let held = self.load(view, p)?;
            let Some(pd) = held.as_deref().and_then(as_dict) else {
                break;
            };
            next = name_step(pd, &mut parts, &mut kind);
        }
        parts.reverse();
        Ok((parts.join("."), kind))
    }

    /// The pages of a revision, walked from its catalog. Every node, kid, content stream and
    /// annotation counts as a visit. An indirect /Kids, /Contents or /Annots array is walked
    /// once: pages that share a /Contents or /Annots array get it for the first of them only,
    /// with one issue.
    fn pages(&mut self, view: View, root: Option<&Object>) -> Result<Pages, Halt> {
        let mut out = Pages::default();
        out.deps.extend(ref_num(root));
        let catalog = self.deref(view, root)?;
        let Some(Ok(Object::Reference(top))) = catalog
            .as_deref()
            .and_then(as_dict)
            .map(|c| c.get(b"Pages"))
        else {
            return Ok(out);
        };
        let mut stack = vec![(*top, 1usize)];
        let mut seen = HashSet::new();
        // An indirect array walked already, and the page that got it.
        let mut arrays: HashMap<ObjectId, u32> = HashMap::new();
        let mut shared: BTreeMap<(&'static str, u32), Vec<u32>> = BTreeMap::new();
        while let Some((id, depth)) = stack.pop() {
            if depth > self.limits.max_depth || !seen.insert(id) {
                continue;
            }
            out.deps.insert(id.0);
            let node = self.load(view, id)?;
            let Some(d) = node.as_deref().and_then(as_dict) else {
                continue;
            };
            let number = out.ids.len() as u32 + 1;
            if !name_is(d, b"Type", b"Page") && d.get(b"Kids").is_ok() {
                let kids = d.get(b"Kids").ok();
                if let Some(Object::Reference(r)) = kids {
                    out.deps.insert(r.0);
                    if arrays.insert(*r, number).is_some() {
                        continue;
                    }
                }
                let held = self.deref(view, kids)?;
                if let Some(Object::Array(kids)) = held.as_deref().or(kids) {
                    self.visit(kids.len())?;
                    for k in kids.iter().rev() {
                        if let Object::Reference(r) = k {
                            stack.push((*r, depth + 1));
                        }
                    }
                }
                continue;
            }
            out.num.entry(id).or_insert(number);
            out.ids.push(id);
            let mut content = Vec::new();
            let mut contents_array = None;
            let contents = d.get(b"Contents").ok();
            let first = match contents {
                Some(Object::Reference(r)) => arrays.get(r).copied(),
                _ => None,
            };
            match first {
                Some(first) => shared.entry(("content", first)).or_default().push(number),
                None => {
                    let held = self.deref(view, contents)?;
                    match (contents, held.as_deref()) {
                        (Some(Object::Reference(r)), Some(Object::Stream(s))) => {
                            content.push((*r, Some(s.content.len())));
                        }
                        (Some(Object::Reference(r)), Some(Object::Array(items))) => {
                            contents_array = Some(r.0);
                            arrays.insert(*r, number);
                            self.content_refs(view, items, &mut content)?;
                        }
                        (Some(Object::Reference(r)), _) => content.push((*r, None)),
                        (Some(Object::Array(items)), _) => {
                            self.content_refs(view, items, &mut content)?
                        }
                        _ => {}
                    }
                }
            }
            for (r, _) in &content {
                out.deps.insert(r.0);
                out.deps.extend(self.loader.length_dep(view, *r));
            }
            out.deps.extend(contents_array);
            let annots = d.get(b"Annots").ok();
            let annots_array = ref_num(annots);
            out.deps.extend(annots_array);
            let first = match annots {
                Some(Object::Reference(r)) => arrays.get(r).copied(),
                _ => None,
            };
            if let Some(first) = first {
                shared
                    .entry(("annotation", first))
                    .or_default()
                    .push(number);
            } else {
                if let Some(Object::Reference(r)) = annots {
                    arrays.insert(*r, number);
                }
                let held = self.deref(view, annots)?;
                if let Some(Object::Array(items)) = held.as_deref().or(annots) {
                    self.visit(items.len())?;
                    for o in items {
                        if let Object::Reference(r) = o {
                            out.annots.entry(*r).or_insert(number);
                        }
                    }
                }
            }
            out.content.push(content);
            out.contents_array.push(contents_array);
            out.annots_array.push(annots_array);
        }
        for ((what, first), pages) in shared {
            let list = page_list(&pages);
            let (verb, own) = if pages.len() == 1 {
                ("page", "shares")
            } else {
                ("pages", "share")
            };
            self.issues.push(Issue::new(
                super::LOCATION,
                IssueKind::Damaged,
                format!(
                    "{verb} {list} {own} the {what} list of page {first}; it is compared for page {first} only"
                ),
            ));
        }
        Ok(out)
    }

    fn content_refs(
        &mut self,
        view: View,
        items: &[Object],
        out: &mut Vec<(ObjectId, Option<usize>)>,
    ) -> Result<(), Halt> {
        self.visit(items.len())?;
        for o in items {
            if let Object::Reference(r) = o {
                let len = match self.load(view, *r)?.as_deref() {
                    Some(Object::Stream(s)) => Some(s.content.len()),
                    _ => None,
                };
                out.push((*r, len));
            }
        }
        Ok(())
    }
}

/// Takes the `/T` and `/FT` of one level of a field and gives its `/Parent`.
fn name_step(
    d: &Dictionary,
    parts: &mut Vec<String>,
    kind: &mut Option<Vec<u8>>,
) -> Option<ObjectId> {
    if let Ok(Object::String(t, _)) = d.get(b"T") {
        parts.push(text_string(t));
    }
    if kind.is_none()
        && let Ok(Object::Name(f)) = d.get(b"FT")
    {
        *kind = Some(f.clone());
    }
    match d.get(b"Parent") {
        Ok(Object::Reference(p)) => Some(*p),
        _ => None,
    }
}

/// The page of an added annotation: its `/P`, else the page that lists it.
fn annot_page(d: &Dictionary, c: &Changed, pages: Option<&Pages>) -> Option<u32> {
    let pages = pages?;
    if let Ok(Object::Reference(p)) = d.get(b"P")
        && let Some(n) = pages.num.get(p)
    {
        return Some(*n);
    }
    c.id.and_then(|id| pages.annots.get(&id).copied())
}

/// Added and removed pages, and pages whose content references or lengths changed.
fn page_words(
    b: &Pages,
    a: &Pages,
    changed: &HashSet<u32>,
    explained: &mut HashSet<u32>,
) -> Vec<String> {
    let mut words = Vec::new();
    for (i, id) in a.ids.iter().enumerate() {
        let n = i + 1;
        let Some(&bn) = b.num.get(id) else {
            words.push(format!("added page {n}"));
            explained.insert(id.0);
            explained.extend(a.content[i].iter().map(|(r, _)| r.0));
            explained.extend(a.contents_array[i]);
            explained.extend(a.annots_array[i]);
            continue;
        };
        let bi = bn as usize - 1;
        let differs = b.content[bi] != a.content[i]
            || a.content[i].iter().any(|(r, _)| changed.contains(&r.0))
            || a.contents_array[i].is_some_and(|r| changed.contains(&r));
        if differs {
            words.push(format!("changed page {n} content"));
            explained.insert(id.0);
            for p in [(b, bi), (a, i)] {
                explained.extend(p.0.content[p.1].iter().map(|(r, _)| r.0));
                explained.extend(p.0.contents_array[p.1]);
            }
        }
    }
    for (i, id) in b.ids.iter().enumerate() {
        if !a.num.contains_key(id) {
            words.push(format!("removed page {}", i + 1));
            explained.insert(id.0);
        }
    }
    words
}
