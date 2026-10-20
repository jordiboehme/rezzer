// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A small reader for the raw file, used before lopdf sees it.
//!
//! It finds every `N G obj` header in file order, reads the dictionary after it (names with
//! `#` escapes decoded, strings and nesting skipped) and steps over stream data, so bytes inside
//! a stream are never taken for objects. It also follows the `startxref` and `/Prev` chain to
//! the cross-reference streams lopdf decodes whatever their type. All work is bounded.

use std::collections::HashSet;

use lopdf::ObjectId;

use super::decode::Filter;
use crate::{Limits, OpenError};

/// How many times the file length the scan may read in total before it gives up.
const WORK_FACTOR: usize = 8;

/// How many bytes before an `obj` keyword may hold its `N G` header.
const HEADER_WINDOW: usize = 256;

/// A stream found in the raw bytes.
#[derive(Debug)]
pub(crate) struct RawStream {
    pub id: ObjectId,
    /// The `/Type` name, `#` escapes decoded.
    pub kind: Option<Vec<u8>>,
    pub filters: Vec<Filter>,
    /// Start and end of the stream data in the file.
    pub data: (usize, usize),
}

impl RawStream {
    pub fn is(&self, kind: &[u8]) -> bool {
        self.kind.as_deref() == Some(kind)
    }
}

/// Counts the bytes the scan reads.
struct Work {
    done: usize,
    max: usize,
}

impl Work {
    fn new(len: usize) -> Work {
        Work {
            done: 0,
            max: len.saturating_mul(WORK_FACTOR).saturating_add(1 << 20),
        }
    }

    fn add(&mut self, n: usize) -> Result<(), OpenError> {
        self.done = self.done.saturating_add(n);
        if self.done > self.max {
            return Err(OpenError::Limit(
                "the file is too irregular to check its object streams".into(),
            ));
        }
        Ok(())
    }
}

/// Every object stream and typed cross-reference stream in the file, in file order.
pub(crate) fn typed_streams(
    bytes: &[u8],
    limits: &Limits,
    stop: &dyn Fn() -> bool,
) -> Result<Vec<RawStream>, OpenError> {
    let mut out = Vec::new();
    let mut work = Work::new(bytes.len());
    let mut from = 0;
    let mut hits = 0usize;
    while let Some(k) = find(bytes, b"obj", from) {
        from = k + 3;
        if bytes
            .get(from)
            .is_some_and(|&c| !is_space(c) && !is_delimiter(c))
        {
            continue;
        }
        let Some(id) = object_header(&bytes[k.saturating_sub(HEADER_WINDOW)..k]) else {
            continue;
        };
        hits += 1;
        if hits.is_multiple_of(1024) && stop() {
            return Err(OpenError::Limit("stopped by the caller".into()));
        }
        if hits > limits.max_items {
            return Err(OpenError::Limit(format!(
                "more than {} objects to check",
                limits.max_items
            )));
        }
        let (found, end) = stream_after(bytes, from, id, limits, &mut work)?;
        from = from.max(end);
        if let Some(found) = found
            && (found.is(b"ObjStm") || found.is(b"XRef"))
        {
            out.push(found);
        }
    }
    Ok(out)
}

/// The streams at `startxref` and every `/Prev` and `/XRefStm` offset, whatever their type.
/// `base` is where `%PDF-` starts; offsets count from there, as in lopdf.
pub(crate) fn xref_chain(
    bytes: &[u8],
    base: usize,
    limits: &Limits,
) -> Result<Vec<RawStream>, OpenError> {
    let mut out = Vec::new();
    let mut work = Work::new(bytes.len());
    let Some(at) = rfind(bytes, b"startxref") else {
        return Ok(out);
    };
    let mut todo: Vec<u64> = read_int(bytes, at + 9).into_iter().collect();
    let mut seen = HashSet::new();
    while let Some(offset) = todo.pop() {
        if !seen.insert(offset) {
            continue;
        }
        if seen.len() > limits.max_items {
            return Err(OpenError::Limit(format!(
                "more than {} cross-reference sections",
                limits.max_items
            )));
        }
        let Some(pos) = usize::try_from(offset)
            .ok()
            .and_then(|o| o.checked_add(base))
        else {
            continue;
        };
        let mut scan = Scan::new(bytes, pos, limits);
        scan.skip();
        let dict = if bytes[scan.i.min(bytes.len())..].starts_with(b"xref") {
            // A classic table: its trailer may point at more sections.
            let Some(t) = find(bytes, b"trailer", scan.i) else {
                continue;
            };
            work.add(t - scan.i)?;
            let mut scan = Scan::new(bytes, t + 7, limits);
            let dict = scan.value(0);
            work.add(scan.i - t)?;
            dict
        } else {
            let Some(found) = stream_at(bytes, pos, limits) else {
                continue;
            };
            let dict = dict_at(bytes, &found, limits);
            out.push(found);
            dict
        };
        if let Some(dict) = dict {
            for key in [b"Prev".as_slice(), b"XRefStm".as_slice()] {
                if let Some(Raw::Int(n)) = dict.get(key)
                    && *n >= 0
                {
                    todo.push(*n as u64);
                }
            }
        }
    }
    Ok(out)
}

/// The stream object whose `N G obj` header starts at `pos` (after white space).
pub(crate) fn stream_at(bytes: &[u8], pos: usize, limits: &Limits) -> Option<RawStream> {
    let mut i = pos;
    while bytes.get(i).is_some_and(|&c| is_space(c)) {
        i += 1;
    }
    let k = find(&bytes[..bytes.len().min(i + HEADER_WINDOW)], b"obj", i)?;
    let id = object_header(&bytes[i..k])?;
    let mut work = Work::new(bytes.len());
    stream_after(bytes, k + 3, id, limits, &mut work).ok()?.0
}

/// The dictionary of a found stream, read again (it is small next to its data).
fn dict_at(bytes: &[u8], found: &RawStream, limits: &Limits) -> Option<Raw> {
    let k = rfind(&bytes[..found.data.0], b"obj")?;
    Scan::new(bytes, k + 3, limits).value(0)
}

/// Reads the dictionary after an `obj` keyword and, if a stream follows, where its data is.
/// Also returns where the scan may go on: after the stream data, or after the dictionary.
fn stream_after(
    bytes: &[u8],
    after_obj: usize,
    id: ObjectId,
    limits: &Limits,
    work: &mut Work,
) -> Result<(Option<RawStream>, usize), OpenError> {
    let mut scan = Scan::new(bytes, after_obj, limits);
    let dict = scan.value(0);
    work.add(scan.i.saturating_sub(after_obj))?;
    let Some(dict @ Raw::Dict(_)) = dict else {
        return Ok((None, after_obj));
    };
    scan.skip();
    if bytes.get(scan.i..scan.i + 6) != Some(b"stream") {
        return Ok((None, scan.i));
    }
    let mut start = scan.i + 6;
    if bytes.get(start) == Some(&b'\r') {
        start += 1;
    }
    if bytes.get(start) == Some(&b'\n') {
        start += 1;
    }
    let by_length = match dict.get(b"Length") {
        Some(Raw::Int(n)) if *n >= 0 => start
            .checked_add(*n as usize)
            .filter(|&end| end <= bytes.len())
            .filter(|&end| {
                let tail = &bytes[end..bytes.len().min(end + 16)];
                find(tail, b"endstream", 0).is_some()
            }),
        _ => None,
    };
    let end = match by_length {
        Some(end) => end,
        None => {
            let Some(mut end) = find(bytes, b"endstream", start) else {
                return Ok((None, scan.i));
            };
            work.add(end - start)?;
            while end > start && matches!(bytes[end - 1], b'\r' | b'\n') {
                end -= 1;
            }
            end
        }
    };
    let kind = match dict.get(b"Type") {
        Some(Raw::Name(n)) => Some(n.clone()),
        _ => None,
    };
    let found = RawStream {
        id,
        kind,
        filters: raw_filters(&dict),
        data: (start, end),
    };
    Ok((Some(found), end))
}

fn read_int(bytes: &[u8], from: usize) -> Option<u64> {
    let mut i = from;
    while bytes.get(i).is_some_and(|&c| is_space(c)) {
        i += 1;
    }
    let start = i;
    while bytes.get(i).is_some_and(u8::is_ascii_digit) && i - start < 20 {
        i += 1;
    }
    std::str::from_utf8(&bytes[start..i]).ok()?.parse().ok()
}

fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}

pub(crate) fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= hay.len() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

fn is_delimiter(c: u8) -> bool {
    matches!(
        c,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0')
}

/// A raw value, as much as the scan needs.
#[derive(Debug)]
enum Raw {
    Name(Vec<u8>),
    Int(i64),
    Array(Vec<Raw>),
    Dict(Vec<(Vec<u8>, Raw)>),
    Other,
}

impl Raw {
    fn get(&self, key: &[u8]) -> Option<&Raw> {
        match self {
            Raw::Dict(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

struct Scan<'a> {
    b: &'a [u8],
    i: usize,
    items: usize,
    max_items: usize,
    max_depth: usize,
}

impl<'a> Scan<'a> {
    fn new(b: &'a [u8], i: usize, limits: &Limits) -> Scan<'a> {
        Scan {
            b,
            i,
            items: 0,
            max_items: limits.max_items,
            max_depth: limits.max_depth,
        }
    }

    fn skip(&mut self) {
        while self.i < self.b.len() {
            let c = self.b[self.i];
            if is_space(c) {
                self.i += 1;
            } else if c == b'%' {
                while self.i < self.b.len() && !matches!(self.b[self.i], b'\r' | b'\n') {
                    self.i += 1;
                }
            } else {
                break;
            }
        }
    }

    fn value(&mut self, depth: usize) -> Option<Raw> {
        self.items += 1;
        if depth > self.max_depth || self.items > self.max_items {
            return None;
        }
        self.skip();
        let b = self.b;
        let c = *b.get(self.i)?;
        match c {
            b'/' => {
                self.i += 1;
                let mut name = Vec::new();
                while let Some(&c) = b.get(self.i) {
                    if is_space(c) || is_delimiter(c) {
                        break;
                    }
                    if c == b'#'
                        && let Some(h) = b.get(self.i + 1..self.i + 3)
                        && let Ok(v) = u8::from_str_radix(std::str::from_utf8(h).ok()?, 16)
                    {
                        name.push(v);
                        self.i += 3;
                        continue;
                    }
                    name.push(c);
                    self.i += 1;
                }
                Some(Raw::Name(name))
            }
            b'<' if b.get(self.i + 1) == Some(&b'<') => {
                self.i += 2;
                let mut pairs = Vec::new();
                loop {
                    self.skip();
                    if b.get(self.i..self.i + 2) == Some(b">>") {
                        self.i += 2;
                        return Some(Raw::Dict(pairs));
                    }
                    let Raw::Name(key) = self.value(depth + 1)? else {
                        return None;
                    };
                    let v = self.value(depth + 1)?;
                    pairs.push((key, v));
                }
            }
            b'<' => {
                let end = find(b, b">", self.i)?;
                self.i = end + 1;
                Some(Raw::Other)
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.skip();
                    if b.get(self.i) == Some(&b']') {
                        self.i += 1;
                        return Some(Raw::Array(items));
                    }
                    items.push(self.value(depth + 1)?);
                }
            }
            b'(' => {
                let mut level = 0usize;
                while let Some(&c) = b.get(self.i) {
                    self.i += 1;
                    match c {
                        b'\\' => self.i += 1,
                        b'(' => level += 1,
                        b')' => {
                            level -= 1;
                            if level == 0 {
                                return Some(Raw::Other);
                            }
                        }
                        _ => {}
                    }
                }
                None
            }
            b'0'..=b'9' | b'+' | b'-' | b'.' => {
                let start = self.i;
                self.i += 1;
                while b
                    .get(self.i)
                    .is_some_and(|c| c.is_ascii_digit() || *c == b'.')
                {
                    self.i += 1;
                }
                let text = std::str::from_utf8(&b[start..self.i]).ok()?;
                let Ok(n) = text.parse::<i64>() else {
                    return Some(Raw::Other);
                };
                // `N G R` is a reference.
                let save = self.i;
                self.skip();
                let g_start = self.i;
                while b.get(self.i).is_some_and(u8::is_ascii_digit) {
                    self.i += 1;
                }
                if self.i > g_start {
                    self.skip();
                    if b.get(self.i) == Some(&b'R')
                        && b.get(self.i + 1)
                            .is_none_or(|c| is_space(*c) || is_delimiter(*c))
                    {
                        self.i += 1;
                        return Some(Raw::Other);
                    }
                }
                self.i = save;
                Some(Raw::Int(n))
            }
            _ => {
                // true, false, null or another keyword.
                let start = self.i;
                while b
                    .get(self.i)
                    .is_some_and(|c| !is_space(*c) && !is_delimiter(*c))
                {
                    self.i += 1;
                }
                if self.i == start {
                    return None;
                }
                Some(Raw::Other)
            }
        }
    }
}

/// Parses `N G` at the end of `before` (the bytes before an `obj` keyword).
fn object_header(before: &[u8]) -> Option<ObjectId> {
    let mut i = before.len();
    let trim = |i: &mut usize| {
        while *i > 0 && is_space(before[*i - 1]) {
            *i -= 1;
        }
    };
    let digits = |i: &mut usize| -> Option<u64> {
        let end = *i;
        while *i > 0 && before[*i - 1].is_ascii_digit() {
            *i -= 1;
        }
        if *i == end || end - *i > 10 {
            return None;
        }
        std::str::from_utf8(&before[*i..end]).ok()?.parse().ok()
    };
    trim(&mut i);
    let g = digits(&mut i)?;
    let had_space = i > 0 && is_space(before[i - 1]);
    trim(&mut i);
    if !had_space {
        return None;
    }
    let n = digits(&mut i)?;
    if i > 0 && !is_space(before[i - 1]) && !is_delimiter(before[i - 1]) {
        return None;
    }
    Some((u32::try_from(n).ok()?, u16::try_from(g).ok()?))
}

fn raw_filters(dict: &Raw) -> Vec<Filter> {
    let names: Vec<&[u8]> = match dict.get(b"Filter") {
        Some(Raw::Name(n)) => vec![n],
        Some(Raw::Array(a)) => a
            .iter()
            .filter_map(|r| match r {
                Raw::Name(n) => Some(n.as_slice()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    let params: Vec<Option<&Raw>> = match dict.get(b"DecodeParms") {
        Some(d @ Raw::Dict(_)) => vec![Some(d)],
        Some(Raw::Array(a)) => a.iter().map(Some).collect(),
        _ => Vec::new(),
    };
    names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let early = !matches!(
                params
                    .get(i)
                    .copied()
                    .flatten()
                    .and_then(|p| p.get(b"EarlyChange")),
                Some(Raw::Int(0))
            );
            Filter::from_name(n, early)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_object_streams_with_escaped_names() {
        let file = b"%PDF-1.5\n7 0 obj\n<< /Type /Obj#53tm /N 2 /First 8 /Length 3 >>\nstream\nabc\nendstream\nendobj\n";
        let found = typed_streams(file, &Limits::default(), &|| false).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, (7, 0));
        assert!(found[0].is(b"ObjStm"));
        assert_eq!(&file[found[0].data.0..found[0].data.1], b"abc");
    }

    #[test]
    fn stream_data_is_not_scanned_for_objects() {
        let file = b"%PDF-1.5\n1 0 obj\n<< /Length 50 >>\nstream\n2 0 obj << /Type /ObjStm >> stream\nxxxxxxxxxx\nendstream\nendobj\n";
        let found = typed_streams(file, &Limits::default(), &|| false).unwrap();
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn headers_are_read() {
        assert_eq!(object_header(b"x\n12 0 "), Some((12, 0)));
        assert_eq!(object_header(b"end"), None);
        assert_eq!(object_header(b"a12 0 "), None);
    }
}
