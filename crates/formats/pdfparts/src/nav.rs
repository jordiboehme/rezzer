// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Pages, page labels, the outline, named destinations and links.

use std::collections::{HashMap, HashSet};

use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::objects::{
    Tree, TreeKey, dict, get, get_dict, get_int, get_name, get_text, number, reference, resolve,
    text_string, walk_tree,
};
use crate::{Issue, IssueKind, Limits, Pdf};

/// One page as the page tree gives it, with the inherited attributes resolved.
#[derive(Debug, Clone)]
pub(crate) struct PageNode {
    pub id: ObjectId,
    media_box: Option<[f32; 4]>,
    crop_box: Option<[f32; 4]>,
    rotate: i64,
    pub resources: Option<Object>,
}

/// A page with its label, size and what it carries.
#[derive(Debug, Clone, PartialEq)]
pub struct PageInfo {
    /// The 1-based physical page number.
    pub number: u32,
    /// The label from `/PageLabels`; `None` when the file has no label for the page.
    pub label: Option<String>,
    /// Width of the visible area (crop box) in points, before rotation.
    pub width: f32,
    /// Height of the visible area in points, before rotation.
    pub height: f32,
    /// Clockwise rotation in degrees: 0, 90, 180 or 270.
    pub rotation: u16,
    /// The page or one of its forms uses a font.
    pub has_fonts: bool,
    /// Images the page draws through its resources.
    pub images: Vec<ImageInfo>,
}

/// An image XObject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    /// Colour components per pixel (1 for gray, masks and indexed, 3 for RGB, 4 for CMYK).
    pub components: u8,
}

/// One outline (bookmark) entry, in reading order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineItem {
    /// 1 for top-level entries.
    pub level: u8,
    pub title: String,
    /// The target page in this file, if the entry points at one.
    pub page: Option<u32>,
    /// The target of a URI action. It is never followed.
    pub uri: Option<String>,
}

/// A link annotation.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub page: u32,
    /// The rectangle as written: lower left x and y, upper right x and y.
    pub rect: [f32; 4],
    pub target: LinkTarget,
}

/// Where a link goes. Nothing here is ever followed or opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// A page of this file.
    Page(u32),
    /// A named destination and the page it names, if it resolves.
    Named(String, Option<u32>),
    Uri(String),
    /// A page in another PDF file.
    Remote {
        file: String,
        page: Option<u32>,
    },
    /// A file or program to start.
    Launch(String),
    /// Another action, by its type (`JavaScript`, `SubmitForm`, `Named` and so on).
    Other(String),
}

fn rect(doc: &Document, obj: Option<&Object>) -> Option<[f32; 4]> {
    let Some(Object::Array(a)) = obj.and_then(|o| resolve(doc, o)) else {
        return None;
    };
    if a.len() != 4 {
        return None;
    }
    let mut out = [0.0; 4];
    for (slot, o) in out.iter_mut().zip(a) {
        *slot = number(resolve(doc, o)?)?;
    }
    Some(out)
}

/// Walks the page tree from the catalog. A cycle is Damaged, depth and item limits are Limit.
/// `/Count` is never trusted.
pub(crate) fn page_tree(doc: &Document, limits: &Limits, issues: &mut Vec<Issue>) -> Vec<PageNode> {
    let mut out = Vec::new();
    let Some(root) = doc
        .trailer
        .get(b"Root")
        .ok()
        .and_then(|r| dict(doc, r))
        .and_then(|c| c.get(b"Pages").ok())
    else {
        issues.push(Issue::new(
            "document",
            IssueKind::Damaged,
            "the catalog has no page tree",
        ));
        return out;
    };
    let mut seen = HashSet::new();
    let inherited = PageNode {
        id: (0, 0),
        media_box: None,
        crop_box: None,
        rotate: 0,
        resources: None,
    };
    walk_pages(
        doc, root, &inherited, 1, limits, &mut seen, issues, &mut out,
    );
    out
}

#[allow(clippy::too_many_arguments)]
fn walk_pages(
    doc: &Document,
    node: &Object,
    parent: &PageNode,
    depth: usize,
    limits: &Limits,
    seen: &mut HashSet<ObjectId>,
    issues: &mut Vec<Issue>,
    out: &mut Vec<PageNode>,
) -> bool {
    if depth > limits.max_depth {
        issues.push(Issue::new(
            "document",
            IssueKind::Limit,
            format!("the page tree is deeper than {} levels", limits.max_depth),
        ));
        return false;
    }
    if seen.len() >= limits.max_items {
        issues.push(Issue::new(
            "document",
            IssueKind::Limit,
            format!("the page tree has more than {} nodes", limits.max_items),
        ));
        return false;
    }
    let Some(id) = reference(node) else {
        issues.push(Issue::new(
            "document",
            IssueKind::Damaged,
            "a page tree node is not an indirect object",
        ));
        return true;
    };
    if !seen.insert(id) {
        issues.push(Issue::new(
            "document",
            IssueKind::Damaged,
            format!("the page tree has a cycle at object {} {}", id.0, id.1),
        ));
        return true;
    }
    let Some(d) = dict(doc, node) else {
        issues.push(Issue::new(
            &format!("object {} {}", id.0, id.1),
            IssueKind::Damaged,
            "a page tree node is missing or not a dictionary",
        ));
        return true;
    };
    let here = PageNode {
        id,
        media_box: rect(doc, d.get(b"MediaBox").ok()).or(parent.media_box),
        crop_box: rect(doc, d.get(b"CropBox").ok()).or(parent.crop_box),
        rotate: get_int(doc, d, b"Rotate").unwrap_or(parent.rotate),
        resources: d
            .get(b"Resources")
            .ok()
            .cloned()
            .or_else(|| parent.resources.clone()),
    };
    let kind = get_name(doc, d, b"Type");
    let is_pages = kind == Some(b"Pages") || (kind.is_none() && d.has(b"Kids"));
    if !is_pages {
        out.push(here);
        return true;
    }
    if let Some(Object::Array(kids)) = get(doc, d, b"Kids") {
        for kid in kids {
            if !walk_pages(doc, kid, &here, depth + 1, limits, seen, issues, out) {
                return false;
            }
        }
    }
    true
}

fn roman(mut v: u64, lower: bool) -> String {
    const TABLE: [(u64, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut s = String::new();
    for (n, r) in TABLE {
        while v >= n {
            s.push_str(r);
            v -= n;
        }
    }
    if lower { s.to_lowercase() } else { s }
}

/// Above this a roman label would be absurdly long; it is written in digits.
const ROMAN_MAX: u64 = 100_000;

/// Above this a letter label (one letter repeated) would be absurdly long.
const LETTERS_MAX: u64 = 26 * 1000;

fn format_label(style: Option<&[u8]>, value: u64) -> String {
    match style {
        Some(b"D") => value.to_string(),
        Some(b"R") if value <= ROMAN_MAX => roman(value, false),
        Some(b"r") if value <= ROMAN_MAX => roman(value, true),
        Some(s @ (b"A" | b"a")) if value <= LETTERS_MAX => {
            // A to Z, then AA to ZZ, then AAA and so on.
            let base = if s == b"A" { b'A' } else { b'a' };
            let letter = char::from(base + ((value - 1) % 26) as u8);
            std::iter::repeat_n(letter, ((value - 1) / 26 + 1) as usize).collect()
        }
        Some(b"R" | b"r" | b"A" | b"a") => value.to_string(),
        _ => String::new(),
    }
}

/// Where one outline walk stands.
struct OutlineWalk<'a> {
    pdf: &'a Pdf,
    names: HashMap<Vec<u8>, &'a Object>,
    seen: HashSet<ObjectId>,
    issues: Vec<Issue>,
    out: Vec<OutlineItem>,
}

impl Pdf {
    /// All pages in order, with labels, sizes, rotation, fonts and images.
    pub fn pages(&self) -> Vec<PageInfo> {
        let mut issues = Vec::new();
        let labels = self.labels(&mut issues);
        let mut out = Vec::with_capacity(self.pages.len());
        for (i, node) in self.pages.iter().enumerate() {
            let number = i as u32 + 1;
            let media = node.media_box.unwrap_or_else(|| {
                issues.push(Issue::new(
                    &format!("page {number}"),
                    IssueKind::Damaged,
                    "the page has no media box; US Letter is assumed",
                ));
                [0.0, 0.0, 612.0, 792.0]
            });
            let visible = node.crop_box.map(|c| intersect(media, c)).unwrap_or(media);
            let rotation = node.rotate.rem_euclid(360);
            let rotation = if rotation % 90 == 0 {
                rotation as u16
            } else {
                issues.push(Issue::new(
                    &format!("page {number}"),
                    IssueKind::Damaged,
                    format!("rotation {} is not a multiple of 90", node.rotate),
                ));
                0
            };
            let mut has_fonts = false;
            let mut images = Vec::new();
            if let Some(res) = &node.resources {
                let mut seen = HashSet::new();
                self.scan_resources(res, 1, &mut seen, &mut has_fonts, &mut images);
            }
            out.push(PageInfo {
                number,
                label: labels.get(i).cloned().flatten(),
                width: (visible[2] - visible[0]).abs(),
                height: (visible[3] - visible[1]).abs(),
                rotation,
                has_fonts,
                images,
            });
        }
        self.note(issues);
        out
    }

    fn scan_resources(
        &self,
        res: &Object,
        depth: usize,
        seen: &mut HashSet<ObjectId>,
        has_fonts: &mut bool,
        images: &mut Vec<ImageInfo>,
    ) {
        let doc = &self.doc;
        if depth > self.limits.max_depth || seen.len() >= self.limits.max_items {
            return;
        }
        if let Some(id) = reference(res)
            && !seen.insert(id)
        {
            return;
        }
        let Some(res) = dict(doc, res) else {
            return;
        };
        if get_dict(doc, res, b"Font").is_some_and(|f| !f.is_empty()) {
            *has_fonts = true;
        }
        let Some(xobjects) = get_dict(doc, res, b"XObject") else {
            return;
        };
        for (_, x) in xobjects.iter() {
            if let Some(id) = reference(x)
                && !seen.insert(id)
            {
                continue;
            }
            let Some(xd) = dict(doc, x) else {
                continue;
            };
            match get_name(doc, xd, b"Subtype") {
                Some(b"Image") => images.push(ImageInfo {
                    width: get_int(doc, xd, b"Width")
                        .unwrap_or(0)
                        .clamp(0, u32::MAX as i64) as u32,
                    height: get_int(doc, xd, b"Height")
                        .unwrap_or(0)
                        .clamp(0, u32::MAX as i64) as u32,
                    components: self.components(xd),
                }),
                Some(b"Form") => {
                    if let Ok(inner) = xd.get(b"Resources") {
                        self.scan_resources(inner, depth + 1, seen, has_fonts, images);
                    }
                }
                _ => {}
            }
        }
    }

    fn components(&self, image: &Dictionary) -> u8 {
        let doc = &self.doc;
        if matches!(get(doc, image, b"ImageMask"), Some(Object::Boolean(true))) {
            return 1;
        }
        let space = match get(doc, image, b"ColorSpace") {
            Some(o) => o,
            // JPX images may carry their colour space inside the data.
            None => return 0,
        };
        let (family, arg) = match space {
            Object::Name(n) => (n.as_slice(), None),
            Object::Array(a) => match a.first().and_then(|o| resolve(doc, o)) {
                Some(Object::Name(n)) => (n.as_slice(), a.get(1)),
                _ => return 0,
            },
            _ => return 0,
        };
        match family {
            b"DeviceGray" | b"CalGray" | b"G" | b"Indexed" | b"I" | b"Separation" | b"Pattern" => 1,
            b"DeviceRGB" | b"CalRGB" | b"RGB" | b"Lab" => 3,
            b"DeviceCMYK" | b"CMYK" => 4,
            b"ICCBased" => arg
                .and_then(|o| dict(doc, o))
                .and_then(|d| get_int(doc, d, b"N"))
                .map(|n| n.clamp(0, 255) as u8)
                .unwrap_or(0),
            b"DeviceN" => match arg.and_then(|o| resolve(doc, o)) {
                Some(Object::Array(names)) => names.len().min(255) as u8,
                _ => 0,
            },
            _ => 0,
        }
    }

    /// The label of every page; `None` where `/PageLabels` gives none.
    fn labels(&self, issues: &mut Vec<Issue>) -> Vec<Option<String>> {
        let n = self.pages.len();
        let doc = &self.doc;
        let Some(root) = self.catalog().and_then(|c| c.get(b"PageLabels").ok()) else {
            return vec![None; n];
        };
        let mut ranges: Vec<(u64, &Object)> =
            walk_tree(doc, root, Tree::Number, &self.limits, "page labels", issues)
                .into_iter()
                .filter_map(|(k, v)| match k {
                    TreeKey::Number(i) if i >= 0 => Some((i as u64, v)),
                    _ => None,
                })
                .collect();
        ranges.sort_by_key(|(start, _)| *start);
        let mut out = Vec::with_capacity(n);
        let mut r = 0usize;
        for i in 0..n as u64 {
            while r + 1 < ranges.len() && ranges[r + 1].0 <= i {
                r += 1;
            }
            let Some(&(start, range)) = ranges.get(r).filter(|(s, _)| *s <= i) else {
                out.push(None);
                continue;
            };
            let d = dict(doc, range);
            let prefix = d.and_then(|d| get_text(doc, d, b"P")).unwrap_or_default();
            let style = d.and_then(|d| get_name(doc, d, b"S"));
            let first = d
                .and_then(|d| get_int(doc, d, b"St"))
                .filter(|s| *s >= 1)
                .unwrap_or(1) as u64;
            let value = first.saturating_add(i - start);
            out.push(Some(format!("{prefix}{}", format_label(style, value))));
        }
        out
    }

    /// Named destinations from the `/Dests` name tree and the older catalog `/Dests`
    /// dictionary.
    fn dest_names(&self, issues: &mut Vec<Issue>) -> Vec<(Vec<u8>, &Object)> {
        let doc = &self.doc;
        let mut out = Vec::new();
        let Some(catalog) = self.catalog() else {
            return out;
        };
        if let Some(tree) = get_dict(doc, catalog, b"Names").and_then(|n| n.get(b"Dests").ok()) {
            for (k, v) in walk_tree(
                doc,
                tree,
                Tree::Name,
                &self.limits,
                "named destinations",
                issues,
            ) {
                if let TreeKey::Name(name) = k {
                    out.push((name, v));
                }
            }
        }
        if let Some(old) = get_dict(doc, catalog, b"Dests") {
            for (k, v) in old.iter() {
                out.push((k.clone(), v));
            }
        }
        out
    }

    /// The page an explicit or named destination points at. `depth` stops named
    /// destinations that point at names.
    fn dest_page(
        &self,
        dest: &Object,
        names: &HashMap<Vec<u8>, &Object>,
        depth: u8,
    ) -> Option<u32> {
        let doc = &self.doc;
        match resolve(doc, dest)? {
            Object::Array(a) => match a.first()? {
                Object::Reference(id) => self.page_number(*id),
                Object::Integer(n) if *n >= 0 && (*n as u64) < self.pages.len() as u64 => {
                    Some(*n as u32 + 1)
                }
                _ => None,
            },
            Object::Dictionary(d) if depth < 4 => {
                self.dest_page(d.get(b"D").ok()?, names, depth + 1)
            }
            Object::Name(n) | Object::String(n, _) if depth < 4 => {
                self.dest_page(names.get(n.as_slice())?, names, depth + 1)
            }
            _ => None,
        }
    }

    fn name_map<'a>(&'a self, issues: &mut Vec<Issue>) -> HashMap<Vec<u8>, &'a Object> {
        let mut map = HashMap::new();
        for (k, v) in self.dest_names(issues) {
            map.entry(k).or_insert(v);
        }
        map
    }

    /// Every named destination with the page it resolves to, in file order.
    pub fn named_destinations(&self) -> Vec<(String, Option<u32>)> {
        let mut issues = Vec::new();
        let list = self.dest_names(&mut issues);
        let map = self.name_map(&mut Vec::new());
        let out = list
            .iter()
            .map(|(name, dest)| (text_string(name), self.dest_page(dest, &map, 1)))
            .collect();
        self.note(issues);
        out
    }

    /// The outline (bookmarks) in reading order, nested entries after their parent.
    pub fn outline(&self) -> Vec<OutlineItem> {
        let mut walk = OutlineWalk {
            pdf: self,
            names: HashMap::new(),
            seen: HashSet::new(),
            issues: Vec::new(),
            out: Vec::new(),
        };
        walk.names = self.name_map(&mut walk.issues);
        let first = self
            .catalog()
            .and_then(|c| get_dict(&self.doc, c, b"Outlines"))
            .and_then(|o| o.get(b"First").ok());
        if let Some(first) = first {
            walk.siblings(first, 1);
        }
        self.note(walk.issues);
        walk.out
    }

    /// Every link annotation on every page, in page order.
    pub fn links(&self) -> Vec<Link> {
        let doc = &self.doc;
        let mut issues = Vec::new();
        let names = self.name_map(&mut issues);
        let mut out = Vec::new();
        for (i, node) in self.pages.iter().enumerate() {
            let page = i as u32 + 1;
            let Some(pd) = doc.objects.get(&node.id).and_then(|o| dict(doc, o)) else {
                continue;
            };
            let Some(Object::Array(annots)) = get(doc, pd, b"Annots") else {
                continue;
            };
            for a in annots {
                if out.len() >= self.limits.max_items {
                    issues.push(Issue::new(
                        "document",
                        IssueKind::Limit,
                        format!("more than {} links", self.limits.max_items),
                    ));
                    self.note(issues);
                    return out;
                }
                let Some(ad) = dict(doc, a) else {
                    continue;
                };
                if get_name(doc, ad, b"Subtype") != Some(b"Link") {
                    continue;
                }
                let target = if let Some(action) = get_dict(doc, ad, b"A") {
                    self.action_target(action, &names)
                } else {
                    ad.get(b"Dest").ok().map(|d| self.goto_target(d, &names))
                };
                let Some(target) = target else {
                    continue;
                };
                out.push(Link {
                    page,
                    rect: rect(doc, ad.get(b"Rect").ok()).unwrap_or([0.0; 4]),
                    target,
                });
            }
        }
        self.note(issues);
        out
    }

    fn goto_target(&self, dest: &Object, names: &HashMap<Vec<u8>, &Object>) -> LinkTarget {
        match resolve(&self.doc, dest) {
            Some(Object::Name(n) | Object::String(n, _)) => {
                LinkTarget::Named(text_string(n), self.dest_page(dest, names, 1))
            }
            _ => match self.dest_page(dest, names, 1) {
                Some(p) => LinkTarget::Page(p),
                None => LinkTarget::Other("GoTo".into()),
            },
        }
    }

    fn action_target(
        &self,
        action: &Dictionary,
        names: &HashMap<Vec<u8>, &Object>,
    ) -> Option<LinkTarget> {
        let doc = &self.doc;
        let kind = get_name(doc, action, b"S")?;
        Some(match kind {
            b"GoTo" => self.goto_target(action.get(b"D").ok()?, names),
            b"URI" => match get(doc, action, b"URI") {
                Some(Object::String(s, _)) => {
                    LinkTarget::Uri(String::from_utf8_lossy(s).into_owned())
                }
                _ => LinkTarget::Other("URI".into()),
            },
            b"GoToR" => LinkTarget::Remote {
                file: self.file_spec(action).unwrap_or_default(),
                page: match get(doc, action, b"D") {
                    Some(Object::Array(a)) => match a.first() {
                        Some(Object::Integer(n)) if *n >= 0 && *n < i64::from(u32::MAX) => {
                            Some(*n as u32 + 1)
                        }
                        _ => None,
                    },
                    _ => None,
                },
            },
            b"Launch" => LinkTarget::Launch(self.file_spec(action).unwrap_or_default()),
            other => LinkTarget::Other(String::from_utf8_lossy(other).into_owned()),
        })
    }

    /// The file name of an action's `/F` (or `/Win`) file specification.
    fn file_spec(&self, action: &Dictionary) -> Option<String> {
        let doc = &self.doc;
        let spec = get(doc, action, b"F")
            .or_else(|| get_dict(doc, action, b"Win").and_then(|w| get(doc, w, b"F")))?;
        match spec {
            Object::String(s, _) => Some(text_string(s)),
            Object::Dictionary(d) => get_text(doc, d, b"UF").or_else(|| get_text(doc, d, b"F")),
            _ => None,
        }
    }
}

impl OutlineWalk<'_> {
    /// Reads `first` and its `/Next` siblings, each followed by its children.
    fn siblings(&mut self, first: &Object, level: usize) {
        let pdf = self.pdf;
        let doc = &pdf.doc;
        let limits = &pdf.limits;
        if level > limits.max_depth || level > usize::from(u8::MAX) {
            self.issues.push(Issue::new(
                "outline",
                IssueKind::Limit,
                format!(
                    "the outline is deeper than {} levels",
                    limits.max_depth.min(255)
                ),
            ));
            return;
        }
        let mut node = Some(first);
        while let Some(current) = node {
            if self.out.len() >= limits.max_items {
                self.issues.push(Issue::new(
                    "outline",
                    IssueKind::Limit,
                    format!("the outline has more than {} entries", limits.max_items),
                ));
                return;
            }
            if let Some(id) = reference(current)
                && !self.seen.insert(id)
            {
                self.issues.push(Issue::new(
                    "outline",
                    IssueKind::Damaged,
                    format!("the outline has a cycle at object {} {}", id.0, id.1),
                ));
                return;
            }
            let Some(d) = dict(doc, current) else {
                return;
            };
            let title = get_text(doc, d, b"Title").unwrap_or_default();
            let (page, uri) = if let Some(action) = get_dict(doc, d, b"A") {
                match pdf.action_target(action, &self.names) {
                    Some(LinkTarget::Page(p)) => (Some(p), None),
                    Some(LinkTarget::Named(_, p)) => (p, None),
                    Some(LinkTarget::Uri(u)) => (None, Some(u)),
                    _ => (None, None),
                }
            } else if let Ok(dest) = d.get(b"Dest") {
                (pdf.dest_page(dest, &self.names, 1), None)
            } else {
                (None, None)
            };
            self.out.push(OutlineItem {
                level: level as u8,
                title,
                page,
                uri,
            });
            if let Ok(child) = d.get(b"First") {
                self.siblings(child, level + 1);
            }
            node = d.get(b"Next").ok();
        }
    }
}

fn intersect(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let norm = |r: [f32; 4]| {
        [
            r[0].min(r[2]),
            r[1].min(r[3]),
            r[0].max(r[2]),
            r[1].max(r[3]),
        ]
    };
    let (a, b) = (norm(a), norm(b));
    let r = [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ];
    if r[2] <= r[0] || r[3] <= r[1] { a } else { r }
}
