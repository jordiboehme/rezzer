// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Decoding of PDF values: text strings, dates, and bounded walks over number and name trees.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

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
    if !(1..=12).contains(&month) || day < 1 || day > days_in(year, month) {
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

/// Characters of a file value an issue or loss quotes.
const QUOTED_CHARS: usize = 80;

/// A value from the file as an issue or a loss quotes it: escaped, cut to its first 80
/// characters with `...` and its length in bytes when it is longer.
pub fn quote(value: &str) -> String {
    match value.char_indices().nth(QUOTED_CHARS) {
        Some((at, _)) => format!("{:?}... ({} bytes)", &value[..at], value.len()),
        None => format!("{value:?}"),
    }
}

/// The days of `month` (1 to 12) in `year` of the Gregorian calendar.
fn days_in(year: u32, month: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        _ => 31,
    }
}

/// Checks an XMP date (ISO 8601 as XMP allows it: `YYYY`, `YYYY-MM`, `YYYY-MM-DD`, then
/// `Thh:mm`, `Thh:mm:ss` or `Thh:mm:ss.s` with `Z`, `+hh:mm`, `-hh:mm` or no offset) and returns
/// it as written when it is one and names a real day. Anything else gives `None`.
pub fn xmp_date(raw: &str) -> Option<String> {
    let s = raw.trim();
    let b = s.as_bytes();
    let digits = |from: usize, n: usize| -> Option<u32> {
        let part = b.get(from..from + n)?;
        if !part.iter().all(u8::is_ascii_digit) {
            return None;
        }
        std::str::from_utf8(part).ok()?.parse().ok()
    };
    let year = digits(0, 4)?;
    let mut at = 4;
    let (mut month, mut day) = (1, 1);
    if b.get(at) == Some(&b'-') {
        month = digits(at + 1, 2)?;
        at += 3;
        if b.get(at) == Some(&b'-') {
            day = digits(at + 1, 2)?;
            at += 3;
        }
    }
    if !(1..=12).contains(&month) || day < 1 || day > days_in(year, month) {
        return None;
    }
    if b.get(at) == Some(&b'T') {
        if at != 10 {
            return None;
        }
        let hour = digits(at + 1, 2)?;
        if b.get(at + 3) != Some(&b':') {
            return None;
        }
        let minute = digits(at + 4, 2)?;
        at += 6;
        let mut second = 0;
        if b.get(at) == Some(&b':') {
            second = digits(at + 1, 2)?;
            at += 3;
            if b.get(at) == Some(&b'.') {
                let n = b[at + 1..]
                    .iter()
                    .take_while(|c| c.is_ascii_digit())
                    .count();
                if n == 0 {
                    return None;
                }
                at += 1 + n;
            }
        }
        if hour > 23 || minute > 59 || second > 60 {
            return None;
        }
        match b.get(at) {
            None => {}
            Some(b'Z') => at += 1,
            Some(b'+' | b'-') => {
                let oh = digits(at + 1, 2)?;
                if b.get(at + 3) != Some(&b':') {
                    return None;
                }
                let om = digits(at + 4, 2)?;
                if oh > 23 || om > 59 {
                    return None;
                }
                at += 6;
            }
            Some(_) => return None,
        }
    }
    (at == b.len()).then(|| s.to_string())
}

/// Texts one accessor reads, each string in an indirect object once: it is decoded and
/// charged the first time and then shared by every entry that names it. The same goes for
/// values the accessor works out from a shared object, such as the target of an action that
/// many links name. A direct string costs each time, as the file holds it each time.
#[derive(Default)]
pub(crate) struct Shared {
    texts: HashMap<(ObjectId, Decode), Option<Arc<str>>>,
    pub(crate) targets: HashMap<ObjectId, Option<crate::LinkTarget>>,
    pub(crate) found: HashMap<ObjectId, Option<Arc<str>>>,
}

/// How the bytes of a string become text.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Decode {
    /// A text string ([`text_string`]); a name as UTF-8.
    Text,
    /// Bytes as UTF-8, invalid sequences replaced (URIs).
    Lossy,
}

/// The budget is spent: the accessor stops.
#[derive(Debug)]
pub(crate) struct Spent;

impl Shared {
    /// The text `obj` is or names: a string or a name. `Ok(None)` when it is neither.
    pub(crate) fn text(
        &mut self,
        doc: &Document,
        obj: &Object,
        decode: Decode,
        budget: &mut TextBudget<'_>,
    ) -> Result<Option<Arc<str>>, Spent> {
        let id = final_id(doc, obj);
        if let Some(id) = id
            && let Some(t) = self.texts.get(&(id, decode))
        {
            return Ok(t.clone());
        }
        let text: Option<Arc<str>> = match resolve(doc, obj) {
            Some(Object::String(s, _)) => Some(match decode {
                Decode::Text => text_string(s).into(),
                Decode::Lossy => String::from_utf8_lossy(s).into(),
            }),
            Some(Object::Name(n)) => Some(String::from_utf8_lossy(n).into()),
            _ => None,
        };
        if !budget.take(text.as_ref().map_or(0, |t| t.len() as u64)) {
            return Err(Spent);
        }
        if let Some(id) = id {
            self.texts.insert((id, decode), text.clone());
        }
        Ok(text)
    }

    /// The text of `key` in `d`, as [`Shared::text`].
    pub(crate) fn text_at(
        &mut self,
        doc: &Document,
        d: &Dictionary,
        key: &[u8],
        budget: &mut TextBudget<'_>,
    ) -> Result<Option<Arc<str>>, Spent> {
        match d.get(key) {
            Ok(obj) => self.text(doc, obj, Decode::Text, budget),
            Err(_) => Ok(None),
        }
    }
}

/// The object a chain of references from `obj` ends at, `None` when `obj` is direct (or the
/// chain is broken). Memos of shared data are keyed by it, so items that each name an object
/// of their own that only refers on to one shared object still share it.
pub(crate) fn final_id(doc: &Document, obj: &Object) -> Option<ObjectId> {
    doc.dereference(obj).ok().and_then(|(id, _)| id)
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
/// Damaged issue, depth and item limits are Limit issues. Each pair and its copied key are
/// charged to `budget`. Each issue names `location`.
pub(crate) fn walk_tree<'a>(
    doc: &'a Document,
    root: &'a Object,
    tree: Tree,
    limits: &Limits,
    budget: &mut TextBudget<'_>,
    location: &str,
    issues: &mut Vec<Issue>,
) -> Vec<(TreeKey, &'a Object)> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut items = 0usize;
    walk_node(
        doc,
        root,
        tree,
        limits,
        &mut (budget, &mut items),
        location,
        issues,
        &mut seen,
        1,
        &mut out,
    );
    out
}

#[allow(clippy::too_many_arguments)]
fn walk_node<'a>(
    doc: &'a Document,
    node: &'a Object,
    tree: Tree,
    limits: &Limits,
    work: &mut (&mut TextBudget<'_>, &mut usize),
    location: &str,
    issues: &mut Vec<Issue>,
    seen: &mut HashSet<ObjectId>,
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
            *work.1 += 1;
            if *work.1 > limits.max_items {
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
            let copied = match &k {
                TreeKey::Name(n) => n.len(),
                TreeKey::Number(_) => 0,
            };
            if !work.0.charge(copied) {
                issues.push(work.0.limit_issue(location, "tree entries"));
                return false;
            }
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
                work,
                location,
                issues,
                seen,
                depth + 1,
                out,
            ) {
                return false;
            }
        }
    }
    true
}

/// A date value: the text as written and, when it parses, ISO 8601.
pub(crate) fn get_date(doc: &Document, d: &Dictionary, key: &[u8]) -> Option<crate::PdfDate> {
    match get(doc, d, key)? {
        Object::String(s, _) => Some(date_of(s)),
        Object::Name(n) => Some(date_of(n)),
        _ => None,
    }
}

/// Characters of a date's text that are kept: a real PDF date is at most 23 long, so a longer
/// text is not a date and only its start is worth showing.
pub(crate) const DATE_RAW_CHARS: usize = 64;

/// A date from the bytes of its string. Bytes past what a date with spaces around it could
/// take are not decoded at all: such a text is no date, and its start is kept.
pub(crate) fn date_of(bytes: &[u8]) -> crate::PdfDate {
    const MOST: usize = 4 * DATE_RAW_CHARS;
    let long = bytes.len() > MOST;
    let raw = text_string(&bytes[..bytes.len().min(MOST)]);
    let iso = if long { None } else { pdf_date(&raw) };
    let raw = match raw.char_indices().nth(DATE_RAW_CHARS) {
        Some((at, _)) => raw[..at].to_string(),
        None => raw,
    };
    crate::PdfDate { raw, iso }
}

/// Fixed cost of one item a reader copies out of the document, as for one parsed value.
pub(crate) const ITEM_BYTES: u64 = 128;

/// What the readers may still copy out of the document: lists, strings and arrays. Data
/// that many entries share would otherwise be copied once per entry. Each item costs
/// [`ITEM_BYTES`] plus its copied bytes. A budget made by [`crate::Pdf`] draws on one pool that
/// every reader shares, so all readers together stay within the object limit; once a charge
/// does not fit, the pool is spent and nothing more is copied.
#[derive(Debug)]
pub(crate) struct TextBudget<'a> {
    pool: Pool<'a>,
}

#[derive(Debug)]
enum Pool<'a> {
    Shared(&'a AtomicU64),
    Own(u64),
}

impl<'a> TextBudget<'a> {
    /// A budget that draws on `pool`, shared with other readers.
    pub(crate) fn shared(pool: &'a AtomicU64) -> TextBudget<'a> {
        TextBudget {
            pool: Pool::Shared(pool),
        }
    }

    /// A budget of its own, for a walk with a separate bound.
    pub(crate) fn own(left: u64) -> TextBudget<'static> {
        TextBudget {
            pool: Pool::Own(left),
        }
    }

    /// Charges one item with `bytes` copied bytes. False when it does not fit; the item must
    /// then be left out and the reader stop.
    pub(crate) fn charge(&mut self, bytes: usize) -> bool {
        self.take(ITEM_BYTES.saturating_add(bytes as u64))
    }

    /// Charges `bytes` copied bytes without the fixed item cost.
    pub(crate) fn take(&mut self, bytes: u64) -> bool {
        match &mut self.pool {
            Pool::Own(left) => {
                if bytes > *left {
                    *left = 0;
                    return false;
                }
                *left -= bytes;
                true
            }
            // A charge that does not fit spends the pool, as for a budget of its own.
            Pool::Shared(pool) => {
                let before = pool.update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                    left.saturating_sub(bytes)
                });
                before >= bytes
            }
        }
    }

    /// The one Limit issue a reader adds when its budget is spent.
    pub(crate) fn limit_issue(&self, location: &str, what: &str) -> Issue {
        Issue::new(
            location,
            IssueKind::Limit,
            format!("the {what} copy more data than the object limit allows; the rest is not read"),
        )
    }
}

/// How many entries one reader may still visit: kids, chain links, array members and the
/// like, `max_items` in all. Shared data costs each time it is visited, so a reader's time is
/// bounded by what it visits and not only by what it keeps.
#[derive(Debug)]
pub(crate) struct Work {
    left: usize,
    max: usize,
    noted: bool,
}

impl Work {
    pub(crate) fn new(limits: &Limits) -> Work {
        Work {
            left: limits.max_items,
            max: limits.max_items,
            noted: false,
        }
    }

    /// Counts `n` visits. False when they do not fit; the reader then stops.
    pub(crate) fn take(&mut self, n: usize) -> bool {
        if n > self.left {
            self.left = 0;
            return false;
        }
        self.left -= n;
        true
    }

    /// The one Limit issue of a reader whose visits ran out; `None` after the first.
    pub(crate) fn limit_issue(&mut self, location: &str, what: &str) -> Option<Issue> {
        if self.noted {
            return None;
        }
        self.noted = true;
        Some(self.issue(location, what))
    }

    /// The Limit issue for a reader that stops at once.
    pub(crate) fn issue(&self, location: &str, what: &str) -> Issue {
        Issue::new(
            location,
            IssueKind::Limit,
            format!(
                "the {what} visit more than {} entries; the rest is not read",
                self.max
            ),
        )
    }
}
