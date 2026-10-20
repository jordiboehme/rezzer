// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The structure tree (tags): roles after the role map, marked content, alternate text and
//! table attributes, and the ParentTree that maps marked content back to its element.

use std::collections::{HashMap, HashSet};

use lopdf::{Dictionary, Object, ObjectId};

use crate::objects::{
    TextBudget, Tree, TreeKey, dict, get, get_dict, get_int, get_name, get_text, reference,
    resolve, text_string, walk_tree,
};
use crate::{Issue, IssueKind, Pdf};

/// The structure tree of a tagged file. Elements are listed in tree order, parents before
/// their children.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StructTree {
    pub elems: Vec<StructElem>,
    /// Indexes into `elems` of the elements right below the root.
    pub roots: Vec<usize>,
}

/// One structure element.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StructElem {
    /// The structure type after `/RoleMap`, for example `H1`, `P`, `TD` or `Figure`.
    pub role: String,
    /// The page of `/Pg`, here or on the nearest parent that has one, else of the first
    /// marked content reference that names one.
    pub page: Option<u32>,
    /// Marked content ids of the element's own content, in order.
    pub mcids: Vec<u32>,
    pub alt: Option<String>,
    pub actual_text: Option<String>,
    pub lang: Option<String>,
    /// `/Headers` of a table cell: the `/ID` of each header cell.
    pub headers: Vec<String>,
    /// `/Scope` of a table header cell: `Row`, `Column` or `Both`.
    pub scope: Option<String>,
    /// 1 unless the cell spans more columns.
    pub col_span: u32,
    /// 1 unless the cell spans more rows.
    pub row_span: u32,
    /// `/ID`, which `headers` of other elements refer to.
    pub id: Option<String>,
    /// Indexes into [`StructTree::elems`].
    pub children: Vec<usize>,
    /// The index of the parent element; `None` for elements right below the root.
    pub parent: Option<usize>,
}

/// The tree read once, with the ParentTree as indexes.
#[derive(Debug, Default)]
pub(crate) struct Built {
    tree: StructTree,
    /// `/StructParents` key of a page to the element of each marked content id.
    parents: HashMap<i64, Vec<Option<usize>>>,
}

/// Most role map steps followed for one role.
const ROLE_HOPS: usize = 32;

const WHERE: &str = "structure tree";

struct Walk<'a> {
    pdf: &'a Pdf,
    role_map: Option<&'a Dictionary>,
    roles: HashMap<Vec<u8>, String>,
    seen: HashSet<ObjectId>,
    index: HashMap<ObjectId, usize>,
    elems: Vec<StructElem>,
    issues: Vec<Issue>,
    /// What the tree may still copy out of the document.
    budget: TextBudget<'a>,
    /// `/K` entries visited so far, bounded by `max_items`.
    work: usize,
    /// Elements reached a second time, and the first of them.
    repeats: usize,
    first_repeat: Option<ObjectId>,
    /// A limit was hit; nothing more is read.
    full: bool,
}

impl Pdf {
    /// The structure tree, or `None` when the file has no `/StructTreeRoot`. It is read on
    /// the first call and kept. A cycle or an element that two parents share is a Damaged
    /// issue; the rest is still read.
    pub fn struct_tree(&self) -> Option<&StructTree> {
        self.built_structure().map(|b| &b.tree)
    }

    /// The element that holds marked content `mcid` of page `page`, as an index into
    /// [`StructTree::elems`], found through the ParentTree.
    pub fn role_of(&self, page: u32, mcid: u32) -> Option<usize> {
        let built = self.built_structure()?;
        let node = self
            .pages
            .get(usize::try_from(page).ok()?.checked_sub(1)?)?;
        let pd = self
            .doc
            .objects
            .get(&node.id)
            .and_then(|o| dict(&self.doc, o))?;
        let key = get_int(&self.doc, pd, b"StructParents")?;
        built
            .parents
            .get(&key)?
            .get(mcid as usize)
            .copied()
            .flatten()
    }

    fn built_structure(&self) -> Option<&Built> {
        self.structure
            .get_or_init(|| self.read_structure())
            .as_ref()
    }

    fn read_structure(&self) -> Option<Built> {
        let doc = &self.doc;
        let root = get_dict(doc, self.catalog()?, b"StructTreeRoot")?;
        let mut walk = Walk {
            pdf: self,
            role_map: get_dict(doc, root, b"RoleMap"),
            roles: HashMap::new(),
            seen: HashSet::new(),
            index: HashMap::new(),
            elems: Vec::new(),
            issues: Vec::new(),
            budget: self.text_budget(),
            work: 0,
            repeats: 0,
            first_repeat: None,
            full: false,
        };
        let mut roots = Vec::new();
        if let Ok(k) = root.get(b"K") {
            for kid in kids(doc, k) {
                if let Some(i) = walk.elem(kid, None, 1) {
                    roots.push(i);
                }
            }
        }
        let parents = walk.parent_tree(root);
        if let Some(id) = walk.first_repeat {
            walk.issues.push(Issue::new(
                WHERE,
                IssueKind::Damaged,
                format!(
                    "{} {} reached twice: the tree has a cycle or a shared element (the first is object {} {})",
                    walk.repeats,
                    if walk.repeats == 1 { "element is" } else { "elements are" },
                    id.0,
                    id.1
                ),
            ));
        }
        let Walk { elems, issues, .. } = walk;
        self.note(issues);
        Some(Built {
            tree: StructTree { elems, roots },
            parents,
        })
    }
}

/// The entries of a `/K` value: one value or an array of them.
fn kids<'a>(doc: &'a lopdf::Document, k: &'a Object) -> Vec<&'a Object> {
    match resolve(doc, k) {
        Some(Object::Array(a)) => a.iter().collect(),
        Some(_) => vec![k],
        None => Vec::new(),
    }
}

impl Walk<'_> {
    /// Reads the element `obj` and its subtree. Returns its index.
    fn elem(&mut self, obj: &Object, page: Option<u32>, depth: usize) -> Option<usize> {
        let pdf = self.pdf;
        let doc = &pdf.doc;
        let limits = &pdf.limits;
        if self.full {
            return None;
        }
        if depth > limits.max_depth {
            self.issues.push(Issue::new(
                WHERE,
                IssueKind::Limit,
                format!(
                    "the structure tree is deeper than {} levels",
                    limits.max_depth
                ),
            ));
            return None;
        }
        let id = reference(obj);
        if let Some(id) = id
            && !self.seen.insert(id)
        {
            self.repeats += 1;
            self.first_repeat.get_or_insert(id);
            return None;
        }
        let d = dict(doc, obj)?;
        let s = get_name(doc, d, b"S")?;
        if self.elems.len() >= limits.max_items {
            self.full = true;
            self.issues.push(Issue::new(
                WHERE,
                IssueKind::Limit,
                format!(
                    "the structure tree has more than {} elements",
                    limits.max_items
                ),
            ));
            return None;
        }
        let page = d
            .get(b"Pg")
            .ok()
            .and_then(reference)
            .and_then(|p| pdf.page_number(p))
            .or(page);
        let mut elem = StructElem {
            role: self.role(s),
            page,
            alt: get_text(doc, d, b"Alt"),
            actual_text: get_text(doc, d, b"ActualText"),
            lang: get_text(doc, d, b"Lang"),
            id: match get(doc, d, b"ID") {
                Some(Object::String(s, _)) => Some(text_string(s)),
                _ => None,
            },
            col_span: 1,
            row_span: 1,
            ..StructElem::default()
        };
        let text = |s: &Option<String>| s.as_ref().map_or(0, String::len);
        let copied = elem.role.len()
            + text(&elem.alt)
            + text(&elem.actual_text)
            + text(&elem.lang)
            + text(&elem.id);
        if !self.budget.charge(copied) || !attributes(pdf, d, &mut elem, &mut self.budget) {
            self.spent();
            return None;
        }
        let index = self.elems.len();
        if let Some(id) = id {
            self.index.insert(id, index);
        }
        self.elems.push(elem);
        let mut children = Vec::new();
        let mut mcids = Vec::new();
        let mut mcr_page = None;
        if let Ok(k) = d.get(b"K") {
            for kid in kids(doc, k) {
                if self.full {
                    break;
                }
                self.work += 1;
                if self.work > limits.max_items {
                    self.full = true;
                    self.issues.push(Issue::new(
                        WHERE,
                        IssueKind::Limit,
                        format!(
                            "the structure tree has more than {} entries; the rest is not read",
                            limits.max_items
                        ),
                    ));
                    break;
                }
                match resolve(doc, kid) {
                    Some(Object::Integer(m)) => {
                        if let Ok(m) = u32::try_from(*m) {
                            if !self.budget.take(4) {
                                self.spent();
                                break;
                            }
                            mcids.push(m);
                        }
                    }
                    Some(Object::Dictionary(kd)) => match get_name(doc, kd, b"Type") {
                        // Some producers leave out /Type on a marked content reference.
                        Some(b"MCR") | None
                            if kd.has(b"MCID") && get_name(doc, kd, b"S").is_none() =>
                        {
                            if let Some(m) =
                                get_int(doc, kd, b"MCID").and_then(|m| u32::try_from(m).ok())
                            {
                                if !self.budget.take(4) {
                                    self.spent();
                                    break;
                                }
                                mcids.push(m);
                            }
                            if mcr_page.is_none() {
                                mcr_page = kd
                                    .get(b"Pg")
                                    .ok()
                                    .and_then(reference)
                                    .and_then(|p| pdf.page_number(p));
                            }
                        }
                        // A reference to an annotation or another object.
                        Some(b"OBJR") => {}
                        _ => {
                            if let Some(c) = self.elem(kid, page, depth + 1) {
                                children.push(c);
                            }
                        }
                    },
                    _ => {}
                }
            }
        }
        for &c in &children {
            self.elems[c].parent = Some(index);
        }
        let elem = &mut self.elems[index];
        elem.children = children;
        elem.mcids = mcids;
        if elem.page.is_none() {
            elem.page = mcr_page;
        }
        Some(index)
    }

    /// The budget is spent: one Limit issue, and nothing more is read.
    fn spent(&mut self) {
        if !self.full {
            self.full = true;
            self.issues
                .push(self.budget.limit_issue(WHERE, "structure elements"));
        }
    }

    /// The role of structure type `s` after the role map.
    fn role(&mut self, s: &[u8]) -> String {
        if let Some(r) = self.roles.get(s) {
            return r.clone();
        }
        let doc = &self.pdf.doc;
        let mut current = s;
        let mut path: Vec<&[u8]> = vec![s];
        if let Some(map) = self.role_map {
            for hop in 0..=ROLE_HOPS {
                let Some(next) = get_name(doc, map, current) else {
                    break;
                };
                if hop == ROLE_HOPS {
                    self.issues.push(Issue::new(
                        WHERE,
                        IssueKind::Limit,
                        format!(
                            "the role map chain from {} is longer than {ROLE_HOPS} steps",
                            String::from_utf8_lossy(s)
                        ),
                    ));
                    break;
                }
                if path.contains(&next) {
                    self.issues.push(Issue::new(
                        WHERE,
                        IssueKind::Damaged,
                        format!(
                            "the role map has a loop at {}",
                            String::from_utf8_lossy(next)
                        ),
                    ));
                    break;
                }
                path.push(next);
                current = next;
            }
        }
        let role = String::from_utf8_lossy(current).into_owned();
        self.roles.insert(s.to_vec(), role.clone());
        role
    }

    /// The ParentTree as indexes: for each `/StructParents` key, the element of each marked
    /// content id.
    fn parent_tree(&mut self, root: &Dictionary) -> HashMap<i64, Vec<Option<usize>>> {
        let pdf = self.pdf;
        let doc = &pdf.doc;
        let mut out = HashMap::new();
        let Ok(tree) = root.get(b"ParentTree") else {
            return out;
        };
        // The rows have their own bound, so a tree whose elements spent the copy budget
        // still maps marked content to the elements that were read.
        let mut left = pdf.limits.max_items;
        let mut own = TextBudget::own(u64::MAX);
        for (k, v) in walk_tree(
            doc,
            tree,
            Tree::Number,
            &pdf.limits,
            &mut own,
            WHERE,
            &mut self.issues,
        ) {
            let TreeKey::Number(key) = k else {
                continue;
            };
            // A single element is the parent of an annotation or a form; only arrays map
            // marked content.
            let Some(Object::Array(a)) = resolve(doc, v) else {
                continue;
            };
            if a.len() > left {
                self.issues.push(Issue::new(
                    WHERE,
                    IssueKind::Limit,
                    format!(
                        "the parent tree has more than {} entries",
                        pdf.limits.max_items
                    ),
                ));
                break;
            }
            left -= a.len();
            let row = a
                .iter()
                .map(|o| reference(o).and_then(|id| self.index.get(&id).copied()))
                .collect();
            out.insert(key, row);
        }
        out
    }
}

/// Table attributes from `/A`: one attribute dictionary, or an array of them with revision
/// numbers in between. The copied headers are charged to `budget`; false when it is spent.
fn attributes(
    pdf: &Pdf,
    d: &Dictionary,
    elem: &mut StructElem,
    budget: &mut TextBudget<'_>,
) -> bool {
    let doc = &pdf.doc;
    let Ok(a) = d.get(b"A") else {
        return true;
    };
    let list: Vec<&Dictionary> = match resolve(doc, a) {
        Some(Object::Array(items)) => items
            .iter()
            .take(pdf.limits.max_items)
            .filter_map(|o| dict(doc, o))
            .collect(),
        Some(_) => dict(doc, a).into_iter().collect(),
        None => Vec::new(),
    };
    for attr in list {
        if get_name(doc, attr, b"O") != Some(b"Table") {
            continue;
        }
        if let Some(Object::Array(h)) = get(doc, attr, b"Headers") {
            let mut headers = Vec::new();
            for o in h {
                if let Some(Object::String(s, _)) = resolve(doc, o) {
                    if !budget.charge(s.len()) {
                        return false;
                    }
                    headers.push(text_string(s));
                }
            }
            elem.headers = headers;
        }
        if let Some(s) = get_name(doc, attr, b"Scope") {
            elem.scope = Some(String::from_utf8_lossy(s).into_owned());
        }
        let span = |key: &[u8]| {
            get_int(doc, attr, key)
                .filter(|n| *n >= 1)
                .map(|n| n.min(i64::from(u32::MAX)) as u32)
        };
        if let Some(n) = span(b"ColSpan") {
            elem.col_span = n;
        }
        if let Some(n) = span(b"RowSpan") {
            elem.row_span = n;
        }
    }
    true
}
