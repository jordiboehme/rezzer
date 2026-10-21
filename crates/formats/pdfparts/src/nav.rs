// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Pages, page labels, the outline, named destinations and links.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::objects::{
    Decode, Shared, Spent, TextBudget, Tree, TreeKey, dict, final_id, get, get_dict, get_int,
    get_name, number, reference, resolve, text_string, walk_tree,
};
use crate::pagetree::ResourcesKey;
use crate::{Issue, IssueKind, Pdf, pagetree};

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
    /// Images the page draws through its resources. Pages that share their resources share
    /// this list.
    pub images: Arc<[ImageInfo]>,
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
    /// Items that name one title object share it.
    pub title: Arc<str>,
    /// The target page in this file, if the entry points at one.
    pub page: Option<u32>,
    /// The target of a URI action. It is never followed.
    pub uri: Option<Arc<str>>,
}

/// A link annotation.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub page: u32,
    /// The rectangle as written: lower left x and y, upper right x and y.
    pub rect: [f32; 4],
    pub target: LinkTarget,
}

/// Where a link goes. Nothing here is ever followed or opened. Texts that many links name
/// through one object are shared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// A page of this file.
    Page(u32),
    /// A named destination and the page it names, if it resolves.
    Named(Arc<str>, Option<u32>),
    Uri(Arc<str>),
    /// A page in another PDF file.
    Remote {
        file: Arc<str>,
        page: Option<u32>,
    },
    /// A file or program to start.
    Launch(Arc<str>),
    /// Another action, by its type (`JavaScript`, `SubmitForm`, `Named` and so on).
    Other(Arc<str>),
}

pub(crate) fn rect(doc: &Document, obj: Option<&Object>) -> Option<[f32; 4]> {
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

/// Longest page label prefix kept; a longer one is cut.
const PREFIX_MAX: usize = 256;

/// "1 page" or "N pages".
pub(crate) fn pages_word(n: usize) -> String {
    if n == 1 {
        "1 page".into()
    } else {
        format!("{n} pages")
    }
}

/// The first page numbers of a list, then how many more there are.
pub(crate) fn page_list(pages: &[u32]) -> String {
    const SHOWN: usize = 20;
    let mut s: Vec<String> = pages.iter().take(SHOWN).map(u32::to_string).collect();
    if pages.len() > SHOWN {
        s.push(format!("and {} more", pages.len() - SHOWN));
    }
    s.join(", ")
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

/// What one call of [`Pdf::pages`] may still do.
struct Scan {
    /// Resource entries (images, forms and other objects) left to visit.
    left: usize,
    /// An entry was not visited because none were left.
    cut: bool,
    /// The page whose resources ran out of entries first.
    cut_at: Option<u32>,
}

/// Where one outline walk stands.
struct OutlineWalk<'a> {
    pdf: &'a Pdf,
    names: HashMap<Vec<u8>, &'a Object>,
    seen: HashSet<ObjectId>,
    issues: Vec<Issue>,
    out: Vec<OutlineItem>,
    budget: TextBudget<'a>,
    shared: Shared,
    /// The budget ran out; nothing more is read.
    spent: bool,
}

impl Pdf {
    /// All pages in order, with labels, sizes, rotation, fonts and images.
    pub fn pages(&self, stop: &dyn Fn() -> bool) -> Vec<PageInfo> {
        let mut issues = Vec::new();
        let labels = self.labels(&mut issues);
        let mut out = Vec::with_capacity(self.pages.len());
        let mut no_box = Vec::new();
        let mut odd_rotation = Vec::new();
        // Pages with the same resources share the result.
        let mut memo: HashMap<ResourcesKey, (bool, Arc<[ImageInfo]>)> = HashMap::new();
        // Resource entries the whole call may still visit. Different resources can name one
        // shared form, so a bound per resources alone would not do.
        let mut scan = Scan {
            left: self.limits.max_items,
            cut: false,
            cut_at: None,
        };
        for (i, node) in self.pages.iter().enumerate() {
            let number = i as u32 + 1;
            if stop() {
                issues.push(Issue::new(
                    "document",
                    IssueKind::Limit,
                    format!("stopped by the caller; pages from {number} on are not listed"),
                ));
                break;
            }
            let media = node.media_box.unwrap_or_else(|| {
                no_box.push(number);
                [0.0, 0.0, 612.0, 792.0]
            });
            let visible = node.crop_box.map(|c| intersect(media, c)).unwrap_or(media);
            let rotation = node.rotate.rem_euclid(360);
            let rotation = if rotation % 90 == 0 {
                rotation as u16
            } else {
                odd_rotation.push(number);
                0
            };
            let (has_fonts, images) = match pagetree::resources_key(&self.doc, node) {
                Some(key) => match memo.get(&key) {
                    Some(found) => found.clone(),
                    None => {
                        let mut has_fonts = false;
                        let mut images = Vec::new();
                        if let Some(res) = pagetree::resources(&self.doc, node) {
                            let mut seen = HashSet::new();
                            self.scan_resources(
                                res,
                                1,
                                &mut seen,
                                &mut scan,
                                &mut has_fonts,
                                &mut images,
                            );
                        }
                        if scan.cut && scan.cut_at.is_none() {
                            scan.cut_at = Some(number);
                        }
                        let found = (has_fonts, Arc::from(images));
                        memo.insert(key, found.clone());
                        found
                    }
                },
                None => (false, Arc::from(Vec::new())),
            };
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
        if let Some(from) = scan.cut_at {
            issues.push(Issue::new(
                "document",
                IssueKind::Limit,
                format!(
                    "the page resources hold more than {} images and forms in all; from page {from} on, images may be missing",
                    self.limits.max_items
                ),
            ));
        }
        if !no_box.is_empty() {
            issues.push(Issue::new(
                "document",
                IssueKind::Damaged,
                format!(
                    "{} have no media box; US Letter is assumed: {}",
                    pages_word(no_box.len()),
                    page_list(&no_box)
                ),
            ));
        }
        if !odd_rotation.is_empty() {
            issues.push(Issue::new(
                "document",
                IssueKind::Damaged,
                format!(
                    "{} have a rotation that is not a multiple of 90; 0 is used: {}",
                    pages_word(odd_rotation.len()),
                    page_list(&odd_rotation)
                ),
            ));
        }
        self.note(issues);
        out
    }

    fn scan_resources(
        &self,
        res: &Object,
        depth: usize,
        seen: &mut HashSet<ObjectId>,
        scan: &mut Scan,
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
            if scan.left == 0 {
                scan.cut = true;
                return;
            }
            scan.left -= 1;
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
                        self.scan_resources(inner, depth + 1, seen, scan, has_fonts, images);
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
        let mut ranges: Vec<(u64, &Object)> = walk_tree(
            doc,
            root,
            Tree::Number,
            &self.limits,
            &mut self.text_budget(),
            "page labels",
            issues,
        )
        .into_iter()
        .filter_map(|(k, v)| match k {
            TreeKey::Number(i) if i >= 0 => Some((i as u64, v)),
            _ => None,
        })
        .collect();
        ranges.sort_by_key(|(start, _)| *start);
        // Each range is decoded once: prefix, style and first value.
        let ranges: Vec<(u64, String, Option<&[u8]>, u64)> = ranges
            .into_iter()
            .map(|(start, range)| {
                let d = dict(doc, range);
                let prefix = d
                    .map(|d| self.label_prefix(d, start, issues))
                    .unwrap_or_default();
                let style = d.and_then(|d| get_name(doc, d, b"S"));
                let first = d
                    .and_then(|d| get_int(doc, d, b"St"))
                    .filter(|s| *s >= 1)
                    .unwrap_or(1) as u64;
                (start, prefix, style, first)
            })
            .collect();
        let mut out = Vec::with_capacity(n);
        let mut r = 0usize;
        for i in 0..n as u64 {
            while r + 1 < ranges.len() && ranges[r + 1].0 <= i {
                r += 1;
            }
            let Some((start, prefix, style, first)) = ranges.get(r).filter(|g| g.0 <= i) else {
                out.push(None);
                continue;
            };
            let value = first.saturating_add(i - start);
            out.push(Some(format!("{prefix}{}", format_label(*style, value))));
        }
        out
    }

    /// The prefix of a label range, cut to [`PREFIX_MAX`] characters. The raw bytes are cut
    /// before they are decoded, so a huge prefix costs no more than a short one.
    fn label_prefix(&self, range: &Dictionary, start: u64, issues: &mut Vec<Issue>) -> String {
        let Some(Object::String(raw, _)) = get(&self.doc, range, b"P") else {
            return String::new();
        };
        // A character takes at most 4 bytes, plus a byte order mark.
        let cap = PREFIX_MAX * 4 + 3;
        let cut_raw = raw.len() > cap;
        let decoded = text_string(&raw[..raw.len().min(cap)]);
        let long = cut_raw || decoded.chars().count() > PREFIX_MAX;
        if !long {
            return decoded;
        }
        issues.push(Issue::new(
            "page labels",
            IssueKind::Limit,
            format!(
                "a label prefix from page {} on is longer than {PREFIX_MAX} characters and is cut",
                start + 1
            ),
        ));
        decoded.chars().take(PREFIX_MAX).collect()
    }

    /// Named destinations from the `/Dests` name tree and the older catalog `/Dests`
    /// dictionary.
    fn dest_names(
        &self,
        budget: &mut TextBudget<'_>,
        issues: &mut Vec<Issue>,
    ) -> Vec<(Vec<u8>, &Object)> {
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
                budget,
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
                if !budget.charge(k.len()) {
                    issues.push(budget.limit_issue("named destinations", "named destinations"));
                    break;
                }
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
        for (k, v) in self.dest_names(&mut self.text_budget(), issues) {
            map.entry(k).or_insert(v);
        }
        map
    }

    /// Every named destination with the page it resolves to, in file order.
    pub fn named_destinations(&self) -> Vec<(String, Option<u32>)> {
        let mut issues = Vec::new();
        let mut budget = self.text_budget();
        let list = self.dest_names(&mut budget, &mut issues);
        let map: HashMap<Vec<u8>, &Object> =
            list.iter().rev().map(|(k, v)| (k.clone(), *v)).collect();
        let mut out = Vec::with_capacity(list.len());
        for (name, dest) in &list {
            // The names were charged once as they were read; the decoded copy is charged again.
            if !budget.take(name.len() as u64) {
                issues.push(budget.limit_issue("named destinations", "named destinations"));
                break;
            }
            out.push((text_string(name), self.dest_page(dest, &map, 1)));
        }
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
            budget: self.text_budget(),
            shared: Shared::default(),
            spent: false,
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

    /// Every link annotation on every page, in page order. A link object listed several times
    /// is listed once, and an /Annots array that pages share counts for the first of them. An
    /// action or string that many links name is read once and shared.
    pub fn links(&self) -> Vec<Link> {
        let doc = &self.doc;
        let mut issues = Vec::new();
        let names = self.name_map(&mut issues);
        let mut out = Vec::new();
        let mut budget = self.text_budget();
        let mut shared = Shared::default();
        let mut seen = HashSet::new();
        self.each_annot("links", &mut issues, |page, _, a| {
            if let Some(r) = reference(a)
                && !seen.insert(r)
            {
                return Ok(());
            }
            let Some(ad) = dict(doc, a) else {
                return Ok(());
            };
            if get_name(doc, ad, b"Subtype") != Some(b"Link") {
                return Ok(());
            }
            let spent = |budget: &TextBudget<'_>| budget.limit_issue("document", "links");
            let target = if let Ok(action) = ad.get(b"A") {
                self.link_action(action, &names, &mut shared, &mut budget)
            } else if let Ok(dest) = ad.get(b"Dest") {
                self.goto_target(dest, &names, &mut shared, &mut budget)
                    .map(Some)
            } else {
                Ok(None)
            };
            let target = match target {
                Ok(Some(t)) => t,
                Ok(None) => return Ok(()),
                Err(Spent) => return Err(spent(&budget)),
            };
            if !budget.charge(0) {
                return Err(spent(&budget));
            }
            out.push(Link {
                page,
                rect: rect(doc, ad.get(b"Rect").ok()).unwrap_or([0.0; 4]),
                target,
            });
            Ok(())
        });
        self.note(issues);
        out
    }

    /// The target of the action `action` is or names; an action object is read once.
    fn link_action(
        &self,
        action: &Object,
        names: &HashMap<Vec<u8>, &Object>,
        shared: &mut Shared,
        budget: &mut TextBudget<'_>,
    ) -> Result<Option<LinkTarget>, Spent> {
        let id = final_id(&self.doc, action);
        if let Some(id) = id
            && let Some(t) = shared.targets.get(&id)
        {
            return Ok(t.clone());
        }
        let target = match dict(&self.doc, action) {
            Some(d) => self.action_target(d, names, shared, budget)?,
            None => None,
        };
        if let Some(id) = id {
            shared.targets.insert(id, target.clone());
        }
        Ok(target)
    }

    fn goto_target(
        &self,
        dest: &Object,
        names: &HashMap<Vec<u8>, &Object>,
        shared: &mut Shared,
        budget: &mut TextBudget<'_>,
    ) -> Result<LinkTarget, Spent> {
        Ok(match resolve(&self.doc, dest) {
            Some(Object::Name(_) | Object::String(..)) => {
                let name = shared
                    .text(&self.doc, dest, Decode::Text, budget)?
                    .unwrap_or_else(|| "".into());
                LinkTarget::Named(name, self.dest_page(dest, names, 1))
            }
            _ => match self.dest_page(dest, names, 1) {
                Some(p) => LinkTarget::Page(p),
                None => LinkTarget::Other("GoTo".into()),
            },
        })
    }

    fn action_target(
        &self,
        action: &Dictionary,
        names: &HashMap<Vec<u8>, &Object>,
        shared: &mut Shared,
        budget: &mut TextBudget<'_>,
    ) -> Result<Option<LinkTarget>, Spent> {
        let doc = &self.doc;
        let Some(kind) = get_name(doc, action, b"S") else {
            return Ok(None);
        };
        Ok(Some(match kind {
            b"GoTo" => match action.get(b"D") {
                Ok(d) => self.goto_target(d, names, shared, budget)?,
                Err(_) => return Ok(None),
            },
            b"URI" => match action.get(b"URI") {
                Ok(u) if matches!(resolve(doc, u), Some(Object::String(..))) => {
                    match shared.text(doc, u, Decode::Lossy, budget)? {
                        Some(u) => LinkTarget::Uri(u),
                        None => LinkTarget::Other("URI".into()),
                    }
                }
                _ => LinkTarget::Other("URI".into()),
            },
            b"GoToR" => LinkTarget::Remote {
                file: self.file_spec(action, shared, budget)?,
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
            b"Launch" => LinkTarget::Launch(self.file_spec(action, shared, budget)?),
            _ => match action.get(b"S") {
                Ok(s) => LinkTarget::Other(
                    shared
                        .text(doc, s, Decode::Text, budget)?
                        .unwrap_or_else(|| "".into()),
                ),
                Err(_) => return Ok(None),
            },
        }))
    }

    /// The file name of an action's `/F` (or `/Win`) file specification; empty when it names
    /// none. A file specification object is read once.
    fn file_spec(
        &self,
        action: &Dictionary,
        shared: &mut Shared,
        budget: &mut TextBudget<'_>,
    ) -> Result<Arc<str>, Spent> {
        let doc = &self.doc;
        let spec = match action.get(b"F") {
            Ok(f) => Some(f),
            Err(_) => get_dict(doc, action, b"Win").and_then(|w| w.get(b"F").ok()),
        };
        let Some(spec) = spec else {
            return Ok("".into());
        };
        let id = final_id(doc, spec);
        if let Some(id) = id
            && let Some(name) = shared.found.get(&id)
        {
            return Ok(name.clone().unwrap_or_else(|| "".into()));
        }
        let name = match resolve(doc, spec) {
            Some(Object::String(..)) => shared.text(doc, spec, Decode::Text, budget)?,
            Some(Object::Dictionary(d)) => match shared.text_at(doc, d, b"UF", budget)? {
                Some(n) => Some(n),
                None => shared.text_at(doc, d, b"F", budget)?,
            },
            _ => None,
        };
        if let Some(id) = id {
            shared.found.insert(id, name.clone());
        }
        Ok(name.unwrap_or_else(|| "".into()))
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
            let title = self.shared.text_at(doc, d, b"Title", &mut self.budget);
            let target = match d.get(b"A") {
                Ok(action) => {
                    pdf.link_action(action, &self.names, &mut self.shared, &mut self.budget)
                }
                Err(_) => Ok(None),
            };
            let (Ok(title), Ok(target)) = (title, target) else {
                self.spent = true;
                self.issues
                    .push(self.budget.limit_issue("outline", "outline entries"));
                return;
            };
            let title = title.unwrap_or_else(|| "".into());
            let (page, uri) = match target {
                Some(LinkTarget::Page(p)) => (Some(p), None),
                Some(LinkTarget::Named(_, p)) => (p, None),
                Some(LinkTarget::Uri(u)) => (None, Some(u)),
                Some(_) => (None, None),
                None => match d.get(b"Dest") {
                    Ok(dest) => (pdf.dest_page(dest, &self.names, 1), None),
                    Err(_) => (None, None),
                },
            };
            if !self.budget.charge(0) {
                self.spent = true;
                self.issues
                    .push(self.budget.limit_issue("outline", "outline entries"));
                return;
            }
            self.out.push(OutlineItem {
                level: level as u8,
                title,
                page,
                uri,
            });
            if let Ok(child) = d.get(b"First") {
                self.siblings(child, level + 1);
                if self.spent {
                    return;
                }
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
