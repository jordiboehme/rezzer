// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The page list in lopdf's order, inherited page attributes and a bounded check of the tree.

use std::collections::HashSet;

use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::nav::rect;
use crate::objects::{dict, get, get_int, reference};
use crate::{Issue, IssueKind, Limits};

/// One page as the page tree gives it, with the inherited attributes resolved.
#[derive(Debug, Clone)]
pub(crate) struct PageNode {
    pub id: ObjectId,
    pub media_box: Option<[f32; 4]>,
    pub crop_box: Option<[f32; 4]>,
    pub rotate: i64,
    /// The node whose `/Resources` the page uses: the page itself or the nearest ancestor
    /// that has them. Pages share it instead of holding a copy.
    pub resources_from: Option<ObjectId>,
}

/// The `/Resources` of a page, read from the node that holds them.
pub(crate) fn resources<'a>(doc: &'a Document, node: &PageNode) -> Option<&'a Object> {
    let owner = doc.objects.get(&node.resources_from?)?;
    dict(doc, owner)?.get(b"Resources").ok()
}

/// What pages with the same resources have in common, for work done once per resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ResourcesKey {
    /// `/Resources` is a reference to this object. Pages may name it from different nodes.
    Object(ObjectId),
    /// `/Resources` is a dictionary held directly by this node.
    Owner(ObjectId),
}

/// The key of a page's resources: the object `/Resources` refers to, or the node that holds
/// them as a dictionary.
pub(crate) fn resources_key(doc: &Document, node: &PageNode) -> Option<ResourcesKey> {
    let owner = node.resources_from?;
    match doc.dereference(resources(doc, node)?) {
        Ok((Some(id), _)) => Some(ResourcesKey::Object(id)),
        _ => Some(ResourcesKey::Owner(owner)),
    }
}

/// The pages and whether the tree is safe to hand to a parser with another page walk.
pub(crate) struct PageTree {
    pub pages: Vec<PageNode>,
    pub unsafe_tree: bool,
}

/// lopdf's own bound on page tree depth.
const LOPDF_DEPTH: usize = 256;

/// The pages in the order lopdf's page walk gives them (depth-first through `/Kids` in order,
/// as other parsers number pages), with inherited attributes read along `/Parent`. A separate
/// bounded walk checks the tree: a cycle, a shared or repeated node, an untyped node, a kid
/// that is not an object or more depth than the limits allow make it unsafe. `/Count` is
/// never trusted.
pub(crate) fn page_tree(doc: &Document, limits: &Limits, issues: &mut Vec<Issue>) -> PageTree {
    let has_root = doc
        .trailer
        .get(b"Root")
        .ok()
        .and_then(|r| dict(doc, r))
        .is_some_and(|c| c.get(b"Pages").is_ok());
    if !has_root {
        issues.push(Issue::new(
            "document",
            IssueKind::Damaged,
            "the catalog has no page tree",
        ));
        return PageTree {
            pages: Vec::new(),
            unsafe_tree: false,
        };
    }
    // lopdf's walk stops after as many steps as the file has objects, so it always ends.
    let mut ids: Vec<ObjectId> = doc
        .page_iter()
        .take(limits.max_items.saturating_add(1))
        .collect();
    if ids.len() > limits.max_items {
        ids.truncate(limits.max_items);
        issues.push(Issue::new(
            "document",
            IssueKind::Limit,
            format!("more than {} pages; the rest is not read", limits.max_items),
        ));
    }
    let pages = ids.iter().map(|&id| inherit(doc, id, limits)).collect();
    let mut check = Check {
        doc,
        limits,
        on_path: HashSet::new(),
        seen: HashSet::new(),
        order: Vec::new(),
        problems: Vec::new(),
        nodes: 0,
    };
    let root = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Pages").ok())
        .and_then(reference);
    match root {
        Some(root) => {
            check.on_path.insert(root);
            check.seen.insert(root);
            if let Some(d) = doc.objects.get(&root).and_then(|o| dict(doc, o)) {
                check.kids(d, 1);
            }
        }
        None => check.problems.push((
            IssueKind::Damaged,
            "the page tree root is not an indirect object".into(),
        )),
    }
    let mut unsafe_tree = !check.problems.is_empty();
    for (kind, what) in check.problems {
        issues.push(Issue::new("document", kind, what));
    }
    if !unsafe_tree && check.order != ids {
        unsafe_tree = true;
        issues.push(Issue::new(
            "document",
            IssueKind::Damaged,
            "a checked walk of the page tree disagrees with lopdf's page order",
        ));
    }
    PageTree { pages, unsafe_tree }
}

/// A page with the attributes it inherits, read up the `/Parent` chain.
fn inherit(doc: &Document, id: ObjectId, limits: &Limits) -> PageNode {
    let mut node = PageNode {
        id,
        media_box: None,
        crop_box: None,
        rotate: 0,
        resources_from: None,
    };
    let mut rotate = None;
    let mut seen = HashSet::new();
    let mut current = doc.objects.get(&id).and_then(|o| dict(doc, o));
    let mut current_id = id;
    seen.insert(id);
    while let Some(d) = current {
        if node.media_box.is_none() {
            node.media_box = rect(doc, d.get(b"MediaBox").ok());
        }
        if node.crop_box.is_none() {
            node.crop_box = rect(doc, d.get(b"CropBox").ok());
        }
        if rotate.is_none() {
            rotate = get_int(doc, d, b"Rotate");
        }
        if node.resources_from.is_none() && d.get(b"Resources").is_ok() {
            node.resources_from = Some(current_id);
        }
        let parent = d
            .get(b"Parent")
            .ok()
            .and_then(reference)
            .filter(|p| seen.insert(*p) && seen.len() <= limits.max_depth);
        current = parent
            .and_then(|p| doc.objects.get(&p))
            .and_then(|o| dict(doc, o));
        if let Some(p) = parent {
            current_id = p;
        }
    }
    node.rotate = rotate.unwrap_or(0);
    node
}

/// The bounded walk that checks the page tree.
struct Check<'a> {
    doc: &'a Document,
    limits: &'a Limits,
    on_path: HashSet<ObjectId>,
    seen: HashSet<ObjectId>,
    order: Vec<ObjectId>,
    problems: Vec<(IssueKind, String)>,
    nodes: usize,
}

impl Check<'_> {
    /// Walks the kids of a Pages node. Returns false when a limit ends the walk.
    fn kids(&mut self, node: &Dictionary, depth: usize) -> bool {
        let doc = self.doc;
        let Some(Object::Array(kids)) = get(doc, node, b"Kids") else {
            return true;
        };
        for kid in kids {
            self.nodes += 1;
            if self.nodes > self.limits.max_items {
                self.problems.push((
                    IssueKind::Limit,
                    format!(
                        "the page tree has more than {} nodes",
                        self.limits.max_items
                    ),
                ));
                return false;
            }
            let Some(id) = reference(kid) else {
                self.problems.push((
                    IssueKind::Damaged,
                    "a page tree kid is not an indirect object".into(),
                ));
                continue;
            };
            let Some(d) = doc.objects.get(&id).and_then(|o| dict(doc, o)) else {
                self.problems.push((
                    IssueKind::Damaged,
                    format!(
                        "page tree kid {} {} is missing or not a dictionary; it counts as an \
                         empty page",
                        id.0, id.1
                    ),
                ));
                continue;
            };
            if self.on_path.contains(&id) {
                self.problems.push((
                    IssueKind::Damaged,
                    format!("the page tree has a cycle at object {} {}", id.0, id.1),
                ));
                continue;
            }
            if !self.seen.insert(id) {
                self.problems.push((
                    IssueKind::Damaged,
                    format!("page tree node {} {} is listed more than once", id.0, id.1),
                ));
                continue;
            }
            match d.get_type().ok() {
                Some(b"Page") => self.order.push(id),
                Some(b"Pages") => {
                    if depth >= self.limits.max_depth.min(LOPDF_DEPTH) {
                        self.problems.push((
                            IssueKind::Limit,
                            format!(
                                "the page tree is deeper than {} levels",
                                self.limits.max_depth.min(LOPDF_DEPTH)
                            ),
                        ));
                        continue;
                    }
                    self.on_path.insert(id);
                    let go_on = self.kids(d, depth + 1);
                    self.on_path.remove(&id);
                    if !go_on {
                        return false;
                    }
                }
                // lopdf counts a node without a /Type name as a page, as hayro does, and
                // skips a node of another type, which hayro counts.
                None => self.problems.push((
                    IssueKind::Damaged,
                    format!(
                        "page tree node {} {} has no /Type and counts as a page",
                        id.0, id.1
                    ),
                )),
                Some(_) => self.problems.push((
                    IssueKind::Damaged,
                    format!(
                        "page tree node {} {} has a /Type other than Page or Pages and is \
                         skipped",
                        id.0, id.1
                    ),
                )),
            }
        }
        true
    }
}
