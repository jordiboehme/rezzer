// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Decoding of PDF values: text strings, dates, and bounded walks over number and name trees.

use std::collections::HashSet;

use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::{Issue, IssueKind, Limits};

/// PDFDocEncoding bytes 0x18 to 0x1F (ISO 32000-1, Annex D).
const PDFDOC_18: [char; 8] = [
    '\u{02D8}', '\u{02C7}', '\u{02C6}', '\u{02D9}', '\u{02DD}', '\u{02DB}', '\u{02DA}', '\u{02DC}',
];

/// PDFDocEncoding bytes 0x80 to 0xA0. 0x9F is undefined.
const PDFDOC_80: [char; 33] = [
    '\u{2022}', '\u{2020}', '\u{2021}', '\u{2026}', '\u{2014}', '\u{2013}', '\u{0192}', '\u{2044}',
    '\u{2039}', '\u{203A}', '\u{2212}', '\u{2030}', '\u{201E}', '\u{201C}', '\u{201D}', '\u{2018}',
    '\u{2019}', '\u{201A}', '\u{2122}', '\u{FB01}', '\u{FB02}', '\u{0141}', '\u{0152}', '\u{0160}',
    '\u{0178}', '\u{017D}', '\u{0131}', '\u{0142}', '\u{0153}', '\u{0161}', '\u{017E}', '\u{FFFD}',
    '\u{20AC}',
];

fn pdfdoc_char(b: u8) -> Option<char> {
    match b {
        0 => None,
        0x18..=0x1F => Some(PDFDOC_18[usize::from(b - 0x18)]),
        0x80..=0xA0 => Some(PDFDOC_80[usize::from(b - 0x80)]),
        0x7F | 0xAD => Some('\u{FFFD}'),
        b'\t' | b'\n' | b'\r' => Some(char::from(b)),
        0x01..=0x1F => Some('\u{FFFD}'),
        _ => Some(char::from(b)),
    }
}

/// Decodes a PDF text string: UTF-16BE with a byte order mark, UTF-8 with a byte order mark,
/// else PDFDocEncoding. Language escapes inside UTF-16 are dropped, and so are NUL bytes.
pub fn text_string(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, u16::from_be_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        // Not allowed by the standard, but written by some producers.
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).replace('\0', "");
    }
    bytes.iter().filter_map(|&b| pdfdoc_char(b)).collect()
}

fn utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> String {
    let mut units = Vec::with_capacity(bytes.len() / 2);
    let mut in_escape = false;
    for pair in bytes.as_chunks::<2>().0 {
        let u = unit(*pair);
        // ISO 32000-2 7.9.2.2: a language code sits between two U+001B.
        if u == 0x1B {
            in_escape = !in_escape;
            continue;
        }
        if !in_escape && u != 0 {
            units.push(u);
        }
    }
    char::decode_utf16(units)
        .map(|c| c.unwrap_or('\u{FFFD}'))
        .collect()
}

/// Turns a PDF date (`D:YYYYMMDDHHmmSSOHH'mm'`) into ISO 8601. Missing parts get the defaults
/// of ISO 32000 (month and day 01, time 00). `Z` stays `Z`; a date without an offset gets
/// none, because its relation to UTC is unknown. Anything else gives `None`.
pub fn pdf_date(raw: &str) -> Option<String> {
    let s = raw.trim();
    let s = s.strip_prefix("D:").unwrap_or(s);
    let b = s.as_bytes();
    let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if !(4..=14).contains(&digits) || digits % 2 != 0 {
        return None;
    }
    let num = |from: usize, default: u32| -> u32 {
        if from + 2 <= digits {
            s[from..from + 2].parse().unwrap_or(default)
        } else {
            default
        }
    };
    let year: u32 = s[..4].parse().ok()?;
    let (month, day) = (num(4, 1), num(6, 1));
    let (hour, minute, second) = (num(8, 0), num(10, 0), num(12, 0));
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut out = format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}");
    let rest = &s[digits..];
    if rest.is_empty() {
        return Some(out);
    }
    let mut chars = rest.chars();
    match chars.next()? {
        'Z' => {
            // Some producers write an offset after Z. Z already says UTC.
            let tail = chars.as_str();
            if !tail.chars().all(|c| c.is_ascii_digit() || c == '\'') {
                return None;
            }
            out.push('Z');
        }
        sign @ ('+' | '-') => {
            let tail = chars.as_str();
            let digits: String = tail.chars().filter(|c| *c != '\'').collect();
            if !tail.chars().all(|c| c.is_ascii_digit() || c == '\'')
                || !(digits.len() == 2 || digits.len() == 4)
            {
                return None;
            }
            let oh: u32 = digits[..2].parse().ok()?;
            let om: u32 = if digits.len() == 4 {
                digits[2..].parse().ok()?
            } else {
                0
            };
            if oh > 23 || om > 59 {
                return None;
            }
            out.push_str(&format!("{sign}{oh:02}:{om:02}"));
        }
        _ => return None,
    }
    Some(out)
}

/// Follows references to the object they name. A missing object gives `None`. lopdf bounds
/// chains of references, so `5 0 obj 5 0 R` ends.
pub(crate) fn resolve<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Object> {
    doc.dereference(obj).ok().map(|(_, o)| o)
}

/// The dictionary `obj` is or points at.
pub(crate) fn dict<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Dictionary> {
    match resolve(doc, obj)? {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

/// The value of `key` in `d`, references followed.
pub(crate) fn get<'a>(doc: &'a Document, d: &'a Dictionary, key: &[u8]) -> Option<&'a Object> {
    resolve(doc, d.get(key).ok()?)
}

pub(crate) fn get_dict<'a>(
    doc: &'a Document,
    d: &'a Dictionary,
    key: &[u8],
) -> Option<&'a Dictionary> {
    dict(doc, d.get(key).ok()?)
}

pub(crate) fn get_name<'a>(doc: &'a Document, d: &'a Dictionary, key: &[u8]) -> Option<&'a [u8]> {
    match get(doc, d, key)? {
        Object::Name(n) => Some(n),
        _ => None,
    }
}

pub(crate) fn get_text(doc: &Document, d: &Dictionary, key: &[u8]) -> Option<String> {
    match get(doc, d, key)? {
        Object::String(s, _) => Some(text_string(s)),
        Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
        _ => None,
    }
}

pub(crate) fn get_int(doc: &Document, d: &Dictionary, key: &[u8]) -> Option<i64> {
    match get(doc, d, key)? {
        Object::Integer(i) => Some(*i),
        Object::Real(r) if r.is_finite() => Some(*r as i64),
        _ => None,
    }
}

pub(crate) fn number(obj: &Object) -> Option<f32> {
    match obj {
        Object::Integer(i) => Some(*i as f32),
        Object::Real(r) if r.is_finite() => Some(*r),
        _ => None,
    }
}

/// The reference `obj` holds, if any.
pub(crate) fn reference(obj: &Object) -> Option<ObjectId> {
    obj.as_reference().ok()
}

/// Which tree a walk reads: number trees keep pairs in `/Nums`, name trees in `/Names`.
#[derive(Clone, Copy)]
pub(crate) enum Tree {
    Number,
    Name,
}

/// One pair of a tree: the key (an integer for number trees, a string for name trees) and its
/// value, references not followed.
pub(crate) enum TreeKey {
    Number(i64),
    Name(Vec<u8>),
}

/// Walks a number or name tree from `root` and returns its pairs in tree order. A cycle is a
/// Damaged issue, depth and item limits are Limit issues. Each issue names `location`.
pub(crate) fn walk_tree<'a>(
    doc: &'a Document,
    root: &'a Object,
    tree: Tree,
    limits: &Limits,
    location: &str,
    issues: &mut Vec<Issue>,
) -> Vec<(TreeKey, &'a Object)> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut items = 0usize;
    walk_node(
        doc, root, tree, limits, location, issues, &mut seen, &mut items, 1, &mut out,
    );
    out
}

#[allow(clippy::too_many_arguments)]
fn walk_node<'a>(
    doc: &'a Document,
    node: &'a Object,
    tree: Tree,
    limits: &Limits,
    location: &str,
    issues: &mut Vec<Issue>,
    seen: &mut HashSet<ObjectId>,
    items: &mut usize,
    depth: usize,
    out: &mut Vec<(TreeKey, &'a Object)>,
) -> bool {
    if depth > limits.max_depth {
        issues.push(Issue::new(
            location,
            IssueKind::Limit,
            format!("tree deeper than {} levels", limits.max_depth),
        ));
        return false;
    }
    if let Some(id) = reference(node)
        && !seen.insert(id)
    {
        issues.push(Issue::new(
            location,
            IssueKind::Damaged,
            format!("the tree has a cycle at object {} {}", id.0, id.1),
        ));
        return true;
    }
    let Some(d) = dict(doc, node) else {
        issues.push(Issue::new(
            location,
            IssueKind::Damaged,
            "a tree node is not a dictionary",
        ));
        return true;
    };
    let key = match tree {
        Tree::Number => b"Nums".as_slice(),
        Tree::Name => b"Names".as_slice(),
    };
    if let Some(Object::Array(pairs)) = get(doc, d, key) {
        for pair in pairs.chunks(2) {
            *items += 1;
            if *items > limits.max_items {
                issues.push(Issue::new(
                    location,
                    IssueKind::Limit,
                    format!("more than {} tree entries", limits.max_items),
                ));
                return false;
            }
            let [k, v] = pair else {
                issues.push(Issue::new(
                    location,
                    IssueKind::Damaged,
                    "a tree key has no value",
                ));
                break;
            };
            let k = match (tree, resolve(doc, k)) {
                (Tree::Number, Some(Object::Integer(i))) => TreeKey::Number(*i),
                (Tree::Name, Some(Object::String(s, _))) => TreeKey::Name(s.clone()),
                (Tree::Name, Some(Object::Name(s))) => TreeKey::Name(s.clone()),
                _ => {
                    issues.push(Issue::new(
                        location,
                        IssueKind::Damaged,
                        "a tree key has the wrong type",
                    ));
                    continue;
                }
            };
            out.push((k, v));
        }
    }
    if let Some(Object::Array(kids)) = get(doc, d, b"Kids") {
        for kid in kids {
            if !walk_node(
                doc,
                kid,
                tree,
                limits,
                location,
                issues,
                seen,
                items,
                depth + 1,
                out,
            ) {
                return false;
            }
        }
    }
    true
}
