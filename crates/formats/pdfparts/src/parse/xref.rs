// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Cross-reference sections (ISO 32000-1, 7.5.4 to 7.5.8): classic tables and cross-reference
//! streams along the `/Prev` chain, with the `/XRefStm` of hybrid files. The newest definition
//! of an object wins. When the chain cannot be read, [`rebuild`] finds the objects with one
//! forward scan of the file.

use std::collections::{HashMap, HashSet};

use lopdf::{Dictionary, Object};

use super::lexer::{Lexer, is_delimiter, is_space, rfind};
use super::object::{finish, head_at};
use super::predict::{last_parms, unpredict};
use super::{Budget, Fail, stopped};
use crate::guard::{BACKSTOP, Measure, cap_words, filters_with, unpack};
use crate::{Issue, IssueKind, Limits, OpenError};

/// Where an object is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entry {
    /// At this position of the file (counted from the first byte), with this generation.
    Normal { pos: usize, generation: u16 },
    /// The `index`th member of object stream `container`.
    Compressed { container: u32, index: u32 },
}

/// What the cross-reference chain gives.
#[derive(Debug, Default)]
pub(crate) struct Chain {
    pub table: HashMap<u32, Entry>,
    /// `/Root`, `/Info`, `/Encrypt` and `/ID` from the newest trailer that has them.
    pub trailer: Dictionary,
    pub issues: Vec<Issue>,
    /// Where each section read so far starts, after white space and comments, and where the
    /// data of each cross-reference stream read so far starts. An offset that leads to one of
    /// them again is skipped, so no section is read or unpacked twice. Every header that
    /// leads to one stream (leading zeros, a header inside the digits of another) shares its
    /// data start, while an update that writes the stream object again puts it elsewhere.
    starts: HashSet<usize>,
    streams: HashSet<usize>,
    /// The chain was cut at the work bound: objects only older sections define are missing
    /// from the table.
    pub cut: bool,
    /// Keep free entries in `freed` (only when a section is read on its own).
    keep_free: bool,
    freed: Vec<u32>,
    /// Where the section read last starts, and where it ends: after its trailer dictionary or
    /// after the data of its cross-reference stream.
    start: Option<usize>,
    end: usize,
}

impl Chain {
    /// Where the objects of the table and the sections read start.
    pub fn starts(&self) -> impl Iterator<Item = usize> + '_ {
        let objects = self.table.values().filter_map(|e| match e {
            Entry::Normal { pos, .. } => Some(*pos),
            Entry::Compressed { .. } => None,
        });
        objects.chain(self.starts.iter().copied())
    }

    fn add(&mut self, n: u32, e: Entry) {
        if n > 0 {
            self.table.entry(n).or_insert(e);
        }
    }

    fn free(&mut self, n: u64) {
        if self.keep_free
            && let Ok(n) = u32::try_from(n)
            && n > 0
        {
            self.freed.push(n);
        }
    }

    fn merge(&mut self, dict: &Dictionary) {
        for key in [b"Root".as_slice(), b"Info", b"Encrypt", b"ID"] {
            if self.trailer.get(key).is_err()
                && let Ok(v) = dict.get(key)
            {
                self.trailer.set(key, v.clone());
            }
        }
    }
}

/// The limits for streams that are unpacked while the file is read: object streams and
/// cross-reference streams. Their cap never depends on the file.
pub(crate) fn tight(limits: &Limits) -> Limits {
    Limits {
        max_stream_bytes: limits.max_stream_bytes.min(BACKSTOP),
        ..*limits
    }
}

/// Reads the chain that starts at the last `startxref`. `Ok(None)` means it is damaged and the
/// table must be rebuilt; a limit ends the open.
pub(crate) fn read_chain(
    bytes: &[u8],
    base: usize,
    budget: &mut Budget,
) -> Result<Option<Chain>, OpenError> {
    let Some(at) = rfind(bytes, b"startxref") else {
        return Ok(None);
    };
    let Some(start) = Lexer::new(bytes, at + 9).uint() else {
        return Ok(None);
    };
    let mut chain = Chain::default();
    let mut seen = HashSet::new();
    let mut next = Some(start);
    // Every section costs one item and its entries and trailer values one each.
    let first_items = budget.items;
    while let Some(off) = next {
        if !seen.insert(off) {
            break;
        }
        let max = budget.limits.max_items;
        if seen.len().saturating_add(budget.items - first_items) > max {
            chain.issues.push(Issue::new(
                "document",
                IssueKind::Limit,
                format!(
                    "the cross-reference chain has more than {max} sections and entries; the sections from offset {off} on are not read, and objects only they define are looked up by a scan of the file"
                ),
            ));
            chain.cut = true;
            break;
        }
        match section(bytes, base, off, budget, &mut chain)? {
            Some(prev) => next = prev,
            None => return Ok(None),
        }
    }
    if chain.trailer.get(b"Root").is_err() {
        return Ok(None);
    }
    Ok(Some(chain))
}

/// The offset the last `startxref` of the file names.
pub(crate) fn last_startxref(bytes: &[u8]) -> Option<u64> {
    let at = rfind(bytes, b"startxref")?;
    Lexer::new(bytes, at + 9).uint()
}

/// One cross-reference section read on its own.
#[derive(Debug)]
pub(crate) struct Part {
    /// Where the section starts (the `xref` keyword or the stream object) and ends.
    pub start: usize,
    pub end: usize,
    /// The entries in use it lists (with those of a hybrid file's `/XRefStm`).
    pub table: HashMap<u32, Entry>,
    /// The object numbers it lists as free.
    pub freed: Vec<u32>,
    /// `/Root`, `/Info`, `/Encrypt` and `/ID` of its trailer.
    pub trailer: Dictionary,
    pub prev: Option<u64>,
}

/// What reading one section gives.
pub(crate) enum Step {
    Part(Part),
    /// The offset leads to a section read already.
    Seen,
    Damaged,
}

/// Reads the sections of a chain one by one. A section is read at most once, however many
/// offsets lead to it.
#[derive(Default)]
pub(crate) struct Sections {
    starts: HashSet<usize>,
    streams: HashSet<usize>,
}

impl Sections {
    pub fn read(
        &mut self,
        bytes: &[u8],
        base: usize,
        off: u64,
        budget: &mut Budget,
    ) -> Result<Step, OpenError> {
        let mut chain = Chain {
            starts: std::mem::take(&mut self.starts),
            streams: std::mem::take(&mut self.streams),
            keep_free: true,
            ..Chain::default()
        };
        let found = section(bytes, base, off, budget, &mut chain);
        self.starts = std::mem::take(&mut chain.starts);
        self.streams = std::mem::take(&mut chain.streams);
        Ok(match found? {
            None => Step::Damaged,
            Some(prev) => match chain.start {
                None => Step::Seen,
                Some(start) => Step::Part(Part {
                    start,
                    end: chain.end.max(start),
                    table: chain.table,
                    freed: chain.freed,
                    trailer: chain.trailer,
                    prev,
                }),
            },
        })
    }
}

/// The places an offset may point at: counted from `%PDF-`, as most readers do, or from the
/// first byte.
pub(crate) fn places(base: usize, off: u64) -> Vec<usize> {
    let Ok(off) = usize::try_from(off) else {
        return Vec::new();
    };
    let mut out = vec![base.saturating_add(off)];
    if base > 0 {
        out.push(off);
    }
    out
}

/// Reads one section. Returns its `/Prev`, or `None` when it is damaged.
fn section(
    bytes: &[u8],
    base: usize,
    off: u64,
    budget: &mut Budget,
    chain: &mut Chain,
) -> Result<Option<Option<u64>>, OpenError> {
    for pos in places(base, off) {
        if pos >= bytes.len() {
            continue;
        }
        let mut l = Lexer::new(bytes, pos);
        l.skip();
        budget.skipped(l.i - pos).map_err(Fail::into_open)?;
        let start = l.i;
        if chain.starts.contains(&start) {
            // Read already through another offset: the chain ends here.
            return Ok(Some(None));
        }
        if l.keyword(b"xref") {
            chain.starts.insert(start);
            chain.start = Some(start);
            return classic(bytes, base, l, budget, chain);
        }
        if let Some(prev) = stream_section(bytes, base, pos, budget, chain)? {
            return Ok(Some(prev));
        }
    }
    Ok(None)
}

fn fail(f: Fail) -> Result<Option<Option<u64>>, OpenError> {
    match f {
        Fail::Bad(_) => Ok(None),
        Fail::Stop(e) => Err(e),
    }
}

fn int(d: &Dictionary, key: &[u8]) -> Option<i64> {
    d.get(key).ok().and_then(|o| o.as_i64().ok())
}

fn classic(
    bytes: &[u8],
    base: usize,
    mut l: Lexer,
    budget: &mut Budget,
    chain: &mut Chain,
) -> Result<Option<Option<u64>>, OpenError> {
    loop {
        if l.keyword(b"trailer") {
            l.skip();
            if !l.b[l.i..].starts_with(b"<<") {
                return Ok(None);
            }
            l.i += 2;
            let trailer = match l.dictionary(budget, 0) {
                Ok(d) => d,
                Err(f) => return fail(f),
            };
            let end = l.i;
            chain.merge(&trailer);
            // A hybrid file lists the members of its object streams in a stream beside the
            // table. Its entries come before older sections.
            if let Some(stm) = int(&trailer, b"XRefStm").and_then(|v| u64::try_from(v).ok()) {
                for pos in places(base, stm) {
                    if pos < bytes.len()
                        && stream_section(bytes, base, pos, budget, chain)?.is_some()
                    {
                        break;
                    }
                }
            }
            chain.end = end;
            return Ok(Some(
                int(&trailer, b"Prev").and_then(|v| u64::try_from(v).ok()),
            ));
        }
        let (Some(first), Some(count)) = (l.uint(), l.uint()) else {
            return Ok(None);
        };
        for k in 0..count {
            let (Some(off), Some(generation)) = (l.uint(), l.uint()) else {
                return Ok(None);
            };
            l.skip();
            let kind = l.regular();
            if kind != b"n" && kind != b"f" {
                return Ok(None);
            }
            if let Err(f) = budget.entry() {
                return fail(f);
            }
            let n = first.saturating_add(k);
            if kind == b"f" {
                chain.free(n);
            }
            if kind == b"n"
                && let (Ok(n), Ok(generation), Ok(pos)) = (
                    u32::try_from(n),
                    u16::try_from(generation),
                    usize::try_from(off),
                )
            {
                chain.add(
                    n,
                    Entry::Normal {
                        pos: base.saturating_add(pos),
                        generation,
                    },
                );
            }
        }
    }
}

/// Reads a cross-reference stream at `pos`. Returns its `/Prev`, or `None` when there is none
/// at `pos`. A stream over a cap ends the open.
fn stream_section(
    bytes: &[u8],
    base: usize,
    pos: usize,
    budget: &mut Budget,
    chain: &mut Chain,
) -> Result<Option<Option<u64>>, OpenError> {
    let mut l = Lexer::new(bytes, pos);
    l.skip();
    budget.skipped(l.i - pos).map_err(Fail::into_open)?;
    let start = l.i;
    if chain.starts.contains(&start) {
        return Ok(Some(None));
    }
    let head = match head_at(bytes, start, budget) {
        Ok(head) => head,
        Err(f) => return fail(f),
    };
    let Some(data_start) = head.stream else {
        return Ok(None);
    };
    if !chain.streams.insert(data_start) {
        return Ok(Some(None));
    }
    chain.starts.insert(start);
    if chain.start.is_none() {
        chain.start = Some(start);
    }
    let (_, obj) = match finish(bytes, head, None, budget) {
        Ok(found) => found,
        Err(f) => return fail(f),
    };
    let Object::Stream(stream) = obj else {
        return Ok(None);
    };
    chain.end = data_start.saturating_add(stream.content.len());
    let dict = &stream.dict;
    // lopdf takes any stream here as a cross-reference stream, typed or not.
    if !(dict.has_type(b"XRef") || dict.get(b"W").is_ok()) {
        return Ok(None);
    }
    let data = match unpack_stream(&stream, budget)? {
        Some(d) => d,
        None => return Ok(None),
    };
    let data = unpredict(data, last_parms(dict));
    let w: Vec<usize> = match dict.get(b"W") {
        Ok(Object::Array(a)) if a.len() == 3 => a
            .iter()
            .map(|o| {
                o.as_i64()
                    .ok()
                    .filter(|v| (0..=8).contains(v))
                    .unwrap_or(99) as usize
            })
            .collect(),
        _ => return Ok(None),
    };
    if w.iter().any(|v| *v > 8) {
        return Ok(None);
    }
    let row: usize = w.iter().sum();
    if row == 0 {
        return Ok(None);
    }
    let size = int(dict, b"Size").unwrap_or(0).max(0);
    let index: Vec<i64> = match dict.get(b"Index") {
        Ok(Object::Array(a)) => a.iter().map(|o| o.as_i64().unwrap_or(0)).collect(),
        _ => vec![0, size],
    };
    let field = |r: &[u8], from: usize, width: usize| -> u64 {
        r[from..from + width]
            .iter()
            .fold(0u64, |v, b| (v << 8) | u64::from(*b))
    };
    let mut rows = data.chunks_exact(row);
    'sub: for pair in index.as_chunks::<2>().0 {
        let (first, count) = (pair[0].max(0) as u64, pair[1].max(0) as u64);
        for k in 0..count {
            let Some(r) = rows.next() else {
                break 'sub;
            };
            if let Err(f) = budget.entry() {
                return fail(f);
            }
            let kind = if w[0] == 0 { 1 } else { field(r, 0, w[0]) };
            let a = field(r, w[0], w[1]);
            let b = field(r, w[0] + w[1], w[2]);
            let Ok(num) = u32::try_from(first.saturating_add(k)) else {
                continue;
            };
            match kind {
                1 => {
                    if let (Ok(pos), Ok(generation)) = (usize::try_from(a), u16::try_from(b)) {
                        chain.add(
                            num,
                            Entry::Normal {
                                pos: base.saturating_add(pos),
                                generation,
                            },
                        );
                    }
                }
                2 => {
                    if let (Ok(container), Ok(index)) = (u32::try_from(a), u32::try_from(b)) {
                        chain.add(num, Entry::Compressed { container, index });
                    }
                }
                0 => chain.free(first.saturating_add(k)),
                _ => {}
            }
        }
    }
    chain.merge(dict);
    Ok(Some(int(dict, b"Prev").and_then(|v| u64::try_from(v).ok())))
}

/// Unpacks an object stream or a cross-reference stream under the fixed cap. `None` when it
/// cannot be decoded; a cap ends the open.
pub(crate) fn unpack_stream(
    stream: &lopdf::Stream,
    budget: &mut Budget,
) -> Result<Option<Vec<u8>>, OpenError> {
    let tight = tight(budget.limits);
    let filters = filters_with(&stream.dict, &|o| Some(o));
    let left = budget
        .limits
        .max_total_bytes
        .saturating_sub(budget.unpacked);
    match unpack(&stream.content, &filters, &tight, left, budget.stop) {
        (Measure::Fits(n), data) => {
            // The unpacked bytes count against the object budget for good, even after the
            // buffer is dropped: freed large buffers may stay resident.
            budget.unpacked += n;
            budget.charge(n).map_err(Fail::into_open)?;
            // Reading the unpacked bytes is work like reading the file.
            budget.more_work(n);
            Ok(data)
        }
        (Measure::Over(cap), _) => Err(OpenError::Limit(format!(
            "an object stream or cross-reference stream {}",
            cap_words(cap, &tight)
        ))),
        (Measure::Stopped, _) => Err(stopped()),
    }
}

/// What a scan of the whole file finds.
#[derive(Debug, Default)]
pub(crate) struct Rebuilt {
    /// Object number to position and generation; the last definition wins.
    pub table: HashMap<u32, (usize, u16)>,
    /// Positions right after each `trailer` keyword.
    pub trailers: Vec<usize>,
}

impl Rebuilt {
    /// Where the objects found and the trailers start.
    pub fn starts(&self) -> impl Iterator<Item = usize> + '_ {
        let objects = self.table.values().map(|&(pos, _)| pos);
        objects.chain(self.trailers.iter().copied())
    }
}

/// Finds every `N G obj` header with one forward pass over the file. White space and comments
/// may stand between the three tokens, and numbers may have leading zeros.
pub(crate) fn rebuild(bytes: &[u8], budget: &mut Budget) -> Result<Rebuilt, OpenError> {
    let mut out = Rebuilt::default();
    // The last two tokens, when they were unsigned integers: value and start.
    let mut prev: [Option<(u64, usize)>; 2] = [None, None];
    let mut i = 0;
    let mut next_poll = 1 << 20;
    while i < bytes.len() {
        if i >= next_poll {
            next_poll += 1 << 20;
            if (budget.stop)() {
                return Err(stopped());
            }
        }
        let c = bytes[i];
        if is_space(c) {
            i += 1;
            continue;
        }
        if c == b'%' {
            while i < bytes.len() && bytes[i] != b'\r' && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if is_delimiter(c) {
            prev = [None, None];
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && !is_space(bytes[i]) && !is_delimiter(bytes[i]) {
            i += 1;
        }
        let run = &bytes[start..i];
        if run.len() <= 10 && run.iter().all(u8::is_ascii_digit) {
            let v = run.iter().fold(0u64, |v, d| v * 10 + u64::from(d - b'0'));
            prev = [prev[1], Some((v, start))];
            continue;
        }
        if run == b"obj"
            && let [Some((n, at)), Some((g, _))] = prev
            && let (Ok(n), Ok(g)) = (u32::try_from(n), u16::try_from(g))
            && n > 0
        {
            budget.entry().map_err(Fail::into_open)?;
            out.table.insert(n, (at, g));
        } else if run == b"trailer" {
            out.trailers.push(i);
        }
        prev = [None, None];
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_finds_odd_headers() {
        let limits = Limits::default();
        let file =
            b"%PDF-1.4\n1 0 obj<<>>endobj\n0002 %c\n0 obj (x) endobj\n1 0 obj 5 endobj trailer<<>>";
        let mut budget = Budget::new(&limits, &|| false, file.len());
        let r = rebuild(file, &mut budget).unwrap();
        assert_eq!(r.table.get(&2).map(|e| e.1), Some(0));
        assert_eq!(r.table[&1].0, 52);
        assert_eq!(r.trailers.len(), 1);
    }
}
