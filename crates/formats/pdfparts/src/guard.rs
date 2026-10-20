// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stream guard.
//!
//! Every stream is decoded once into a counting sink that keeps no data. A stream may not
//! unpack to more than `max_stream_bytes` or more than `max_ratio` times its raw length, and
//! all streams together may not unpack to more than `max_total_bytes`. The sink stops at the
//! first byte over a cap, so a bomb costs time up to the cap and almost no memory.
//!
//! lopdf inflates object streams and cross-reference streams while it loads a file. So before
//! lopdf sees the bytes, [`prescan`] finds those streams in the raw file and measures them with
//! the same caps. Everything else is measured by [`measure`] after the load, on the decrypted
//! objects.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};

use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::objects::{get, resolve};
use crate::{Issue, IssueKind, Limits, OpenError};

/// How often, in output bytes, the sink asks the stop callback.
const STOP_EVERY: u64 = 1 << 20;

/// Size of the scratch buffers of the decoding stages.
const CHUNK: usize = 32 * 1024;

/// One decoding step of a stream's filter chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Filter {
    Flate,
    Lzw {
        early_change: bool,
    },
    Ascii85,
    AsciiHex,
    RunLength,
    Brotli,
    /// Data that stays encoded (images, or a filter nobody here decodes). The chain ends.
    Opaque,
    /// A crypt filter. The objects are already decrypted, so it passes data through.
    Crypt,
}

impl Filter {
    fn from_name(name: &[u8], early_change: bool) -> Filter {
        match name {
            b"FlateDecode" | b"Fl" => Filter::Flate,
            b"LZWDecode" | b"LZW" => Filter::Lzw { early_change },
            b"ASCII85Decode" | b"A85" => Filter::Ascii85,
            b"ASCIIHexDecode" | b"AHx" => Filter::AsciiHex,
            b"RunLengthDecode" | b"RL" => Filter::RunLength,
            b"BrotliDecode" => Filter::Brotli,
            b"Crypt" => Filter::Crypt,
            _ => Filter::Opaque,
        }
    }
}

/// What measuring one stream found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Measure {
    /// The stream unpacks to this many bytes (or this many before it broke off).
    Fits(u64),
    /// The stream unpacks past a cap.
    Over(Cap),
    /// The caller asked to stop.
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cap {
    Stream,
    Ratio,
    Total,
}

struct Shared<'a> {
    count: Cell<u64>,
    cap: u64,
    over: Cell<bool>,
    stopped: Cell<bool>,
    next_check: Cell<u64>,
    stop: &'a dyn Fn() -> bool,
}

/// The end of every chain: counts bytes and keeps none.
struct Sink<'s> {
    shared: &'s Shared<'s>,
}

impl Write for Sink<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let s = self.shared;
        let n = s.count.get().saturating_add(buf.len() as u64);
        s.count.set(n);
        if n > s.cap {
            s.over.set(true);
            return Err(io::Error::other("over the cap"));
        }
        if n >= s.next_check.get() {
            s.next_check.set(n.saturating_add(STOP_EVERY));
            if (s.stop)() {
                s.stopped.set(true);
                return Err(io::Error::other("stopped"));
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// LZW (ISO 32000-1 7.4.4) as a writer stage.
struct LzwStage<W: Write> {
    decoder: weezl::decode::Decoder,
    next: W,
    buf: Box<[u8]>,
    done: bool,
}

impl<W: Write> LzwStage<W> {
    fn new(next: W, early_change: bool) -> Self {
        use weezl::BitOrder;
        let decoder = if early_change {
            weezl::decode::Decoder::with_tiff_size_switch(BitOrder::Msb, 8)
        } else {
            weezl::decode::Decoder::new(BitOrder::Msb, 8)
        };
        LzwStage {
            decoder,
            next,
            buf: vec![0; CHUNK].into_boxed_slice(),
            done: false,
        }
    }

    fn pump(&mut self, mut input: &[u8]) -> io::Result<()> {
        while !self.done {
            let r = self.decoder.decode_bytes(input, &mut self.buf);
            self.next.write_all(&self.buf[..r.consumed_out])?;
            input = &input[r.consumed_in..];
            match r.status {
                Ok(weezl::LzwStatus::Done) => self.done = true,
                Ok(_) if r.consumed_in == 0 && r.consumed_out == 0 => break,
                Ok(_) => {}
                // lopdf keeps what came out before an error.
                Err(_) => self.done = true,
            }
        }
        Ok(())
    }
}

impl<W: Write> Write for LzwStage<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pump(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.pump(&[])?;
        self.next.flush()
    }
}

/// ASCIIHexDecode as a writer stage.
struct HexStage<W: Write> {
    next: W,
    high: Option<u8>,
    done: bool,
}

impl<W: Write> Write for HexStage<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut out = Vec::with_capacity(buf.len() / 2 + 1);
        for &ch in buf {
            if self.done {
                break;
            }
            if ch == b'>' {
                self.done = true;
                break;
            }
            if ch.is_ascii_whitespace() {
                continue;
            }
            let Some(nibble) = (ch as char).to_digit(16) else {
                self.done = true;
                break;
            };
            match self.high.take() {
                Some(h) => out.push((h << 4) | nibble as u8),
                None => self.high = Some(nibble as u8),
            }
        }
        self.next.write_all(&out)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(h) = self.high.take() {
            self.next.write_all(&[h << 4])?;
        }
        self.next.flush()
    }
}

/// ASCII85Decode as a writer stage.
struct A85Stage<W: Write> {
    next: W,
    group: u64,
    count: usize,
    done: bool,
}

impl<W: Write> Write for A85Stage<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut out = Vec::with_capacity(buf.len());
        for &ch in buf {
            if self.done {
                break;
            }
            match ch {
                b'~' => self.done = true,
                b'z' if self.count == 0 => out.extend_from_slice(&[0; 4]),
                c if c.is_ascii_whitespace() => {}
                b'!'..=b'u' => {
                    self.group = self.group * 85 + u64::from(ch - b'!');
                    self.count += 1;
                    if self.count == 5 {
                        out.extend_from_slice(&(self.group as u32).to_be_bytes());
                        self.group = 0;
                        self.count = 0;
                    }
                }
                _ => self.done = true,
            }
        }
        self.next.write_all(&out)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.count > 1 {
            let mut g = self.group;
            for _ in self.count..5 {
                g = g * 85 + 84;
            }
            let bytes = (g as u32).to_be_bytes();
            self.next.write_all(&bytes[..self.count - 1])?;
        }
        self.count = 0;
        self.next.flush()
    }
}

/// RunLengthDecode as a writer stage.
struct RunStage<W: Write> {
    next: W,
    /// Literal bytes still to copy.
    literal: usize,
    /// A repeat count waiting for its byte.
    repeat: Option<usize>,
    done: bool,
}

impl<W: Write> Write for RunStage<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut i = 0;
        while i < buf.len() && !self.done {
            if self.literal > 0 {
                let n = self.literal.min(buf.len() - i);
                self.next.write_all(&buf[i..i + n])?;
                self.literal -= n;
                i += n;
            } else if let Some(n) = self.repeat.take() {
                self.next.write_all(&[buf[i]; 128][..n])?;
                i += 1;
            } else {
                match buf[i] {
                    128 => self.done = true,
                    l @ 0..=127 => self.literal = usize::from(l) + 1,
                    l => self.repeat = Some(257 - usize::from(l)),
                }
                i += 1;
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.next.flush()
    }
}

/// Input a Flate stage keeps until its first output byte, so it can replay a stream with a
/// broken zlib header as raw deflate. No output after this much input counts as over the cap:
/// lopdf would keep inflating, and the guard cannot keep buffering.
const PROBE_MAX: usize = 64 * 1024;

/// FlateDecode as a writer stage. Like lopdf, a zlib stream that fails before any output is
/// tried again as raw deflate from its third byte, at any place in the chain.
struct FlateStage<'s, W: Write> {
    inflater: flate2::Decompress,
    zlib: bool,
    next: W,
    buf: Box<[u8]>,
    probe: Option<Vec<u8>>,
    done: bool,
    shared: &'s Shared<'s>,
}

impl<'s, W: Write> FlateStage<'s, W> {
    fn new(next: W, shared: &'s Shared<'s>) -> Self {
        FlateStage {
            inflater: flate2::Decompress::new(true),
            zlib: true,
            next,
            buf: vec![0; CHUNK].into_boxed_slice(),
            probe: Some(Vec::new()),
            done: false,
            shared,
        }
    }

    fn feed(&mut self, mut input: &[u8]) -> io::Result<()> {
        while !self.done {
            let (in0, out0) = (self.inflater.total_in(), self.inflater.total_out());
            let res = self
                .inflater
                .decompress(input, &mut self.buf, flate2::FlushDecompress::None);
            let used = (self.inflater.total_in() - in0) as usize;
            let made = (self.inflater.total_out() - out0) as usize;
            input = &input[used.min(input.len())..];
            if made > 0 {
                self.probe = None;
                self.next.write_all(&self.buf[..made])?;
            }
            match res {
                Ok(flate2::Status::StreamEnd) => self.done = true,
                Ok(_) if used == 0 && made == 0 => break,
                Ok(_) => {}
                Err(_) => {
                    match self.probe.take() {
                        Some(probe) if self.zlib && probe.len() > 2 => {
                            self.zlib = false;
                            self.inflater = flate2::Decompress::new(false);
                            return self.feed(&probe[2..]);
                        }
                        // lopdf keeps what came out before an error.
                        _ => self.done = true,
                    }
                }
            }
        }
        Ok(())
    }
}

impl<W: Write> Write for FlateStage<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(probe) = self.probe.as_mut() {
            probe.extend_from_slice(buf);
            if probe.len() > PROBE_MAX {
                self.shared.over.set(true);
                return Err(io::Error::other("no output from a long deflate stream"));
            }
        }
        self.feed(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.feed(&[])?;
        self.next.flush()
    }
}

/// Passes data on unchanged.
struct Pass<W: Write>(W);

impl<W: Write> Write for Pass<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

fn chain<'s>(filters: &[Filter], shared: &'s Shared<'s>) -> Box<dyn Write + 's> {
    let mut w: Box<dyn Write + 's> = Box::new(Sink { shared });
    for f in filters.iter().rev() {
        w = match f {
            Filter::Flate => Box::new(FlateStage::new(w, shared)),
            Filter::Lzw { early_change } => Box::new(LzwStage::new(w, *early_change)),
            Filter::Ascii85 => Box::new(A85Stage {
                next: w,
                group: 0,
                count: 0,
                done: false,
            }),
            Filter::AsciiHex => Box::new(HexStage {
                next: w,
                high: None,
                done: false,
            }),
            Filter::RunLength => Box::new(RunStage {
                next: w,
                literal: 0,
                repeat: None,
                done: false,
            }),
            Filter::Brotli => Box::new(brotli_decompressor::DecompressorWriter::new(w, 4096)),
            Filter::Crypt | Filter::Opaque => Box::new(Pass(w)),
        };
    }
    w
}

/// Feeds `raw` through the chain. Decoding errors end a stage quietly, as in lopdf; only the
/// sink (over a cap, or stopped) ends the run early.
fn run<'s>(raw: &[u8], filters: &[Filter], shared: &'s Shared<'s>) {
    let mut w = chain(filters, shared);
    for piece in raw.chunks(CHUNK) {
        if w.write_all(piece).is_err() {
            return;
        }
    }
    let _ = w.flush();
}

/// Decodes `raw` through `filters` into a counting sink. `budget` is what is left of the total.
pub(crate) fn measure_bytes(
    raw: &[u8],
    filters: &[Filter],
    limits: &Limits,
    budget: u64,
    stop: &dyn Fn() -> bool,
) -> Measure {
    // The chain ends at the first filter whose output stays encoded.
    let end = filters
        .iter()
        .position(|f| *f == Filter::Opaque)
        .unwrap_or(filters.len());
    let filters = &filters[..end];
    let ratio_cap = limits.max_ratio.saturating_mul((raw.len() as u64).max(1));
    let cap = limits.max_stream_bytes.min(ratio_cap).min(budget);
    let shared = Shared {
        count: Cell::new(0),
        cap,
        over: Cell::new(false),
        stopped: Cell::new(false),
        next_check: Cell::new(STOP_EVERY),
        stop,
    };
    run(raw, filters, &shared);
    if shared.stopped.get() {
        return Measure::Stopped;
    }
    if shared.over.get() {
        let cap = if cap == budget && budget < limits.max_stream_bytes.min(ratio_cap) {
            Cap::Total
        } else if ratio_cap < limits.max_stream_bytes {
            Cap::Ratio
        } else {
            Cap::Stream
        };
        return Measure::Over(cap);
    }
    Measure::Fits(shared.count.get())
}

fn cap_words(cap: Cap, limits: &Limits) -> String {
    match cap {
        Cap::Stream => format!("unpacks to more than {} bytes", limits.max_stream_bytes),
        Cap::Ratio => format!("unpacks to more than {} times its size", limits.max_ratio),
        Cap::Total => format!(
            "would take the document past {} unpacked bytes",
            limits.max_total_bytes
        ),
    }
}

/// The filter chain of a loaded stream.
pub(crate) fn stream_filters(doc: &Document, dict: &Dictionary) -> Vec<Filter> {
    let names: Vec<Vec<u8>> = match get(doc, dict, b"Filter") {
        Some(Object::Name(n)) => vec![n.clone()],
        Some(Object::Array(a)) => a
            .iter()
            .filter_map(|o| match resolve(doc, o) {
                Some(Object::Name(n)) => Some(n.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    let params: Vec<Option<&Dictionary>> = match get(doc, dict, b"DecodeParms") {
        Some(Object::Dictionary(d)) => vec![Some(d)],
        Some(Object::Array(a)) => a
            .iter()
            .map(|o| match resolve(doc, o) {
                Some(Object::Dictionary(d)) => Some(d),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let early = params
                .get(i)
                .copied()
                .flatten()
                .and_then(|p| p.get(b"EarlyChange").ok())
                .and_then(|o| o.as_i64().ok())
                .is_none_or(|v| v != 0);
            Filter::from_name(n, early)
        })
        .collect()
}

/// What the raw scan measured before lopdf loaded the file.
#[derive(Debug, Default)]
pub(crate) struct Prescan {
    /// Object streams and cross-reference streams already charged: object, raw length.
    pub measured: HashMap<ObjectId, u64>,
    pub total: u64,
    /// The longest raw object or cross-reference stream seen, measured or not.
    pub largest_raw: u64,
}

/// Finds `/Type /ObjStm` and `/Type /XRef` streams in the raw bytes and measures them. In a
/// file that looks encrypted, an object stream may hold ciphertext: one over a cap still ends
/// the open, but one that fits is not charged, so [`measure`] measures its plain text later.
pub(crate) fn prescan(
    bytes: &[u8],
    limits: &Limits,
    stop: &dyn Fn() -> bool,
) -> Result<Prescan, OpenError> {
    let mut out = Prescan::default();
    let encrypted = find(bytes, b"/Encrypt", 0).is_some();
    let mut starts = HashSet::new();
    let mut hits = 0usize;
    for needle in [b"/ObjStm".as_slice(), b"/XRef".as_slice()] {
        let mut from = 0;
        while let Some(at) = find(bytes, needle, from) {
            from = at + needle.len();
            if bytes
                .get(from)
                .is_some_and(|&c| !is_delimiter(c) && !c.is_ascii_whitespace())
            {
                continue;
            }
            hits += 1;
            if hits > limits.max_items {
                return Err(OpenError::Limit(format!(
                    "more than {} object streams",
                    limits.max_items
                )));
            }
            if stop() {
                return Err(OpenError::Limit("stopped by the caller".into()));
            }
            let Some(found) = raw_stream_at(bytes, at, limits) else {
                continue;
            };
            if !starts.insert(found.data.0) {
                continue;
            }
            let is_objstm = found.kind == b"ObjStm";
            let raw = &bytes[found.data.0..found.data.1];
            out.largest_raw = out.largest_raw.max(raw.len() as u64);
            let budget = limits.max_total_bytes.saturating_sub(out.total);
            match measure_bytes(raw, &found.filters, limits, budget, stop) {
                Measure::Fits(_) if is_objstm && encrypted => {}
                Measure::Fits(n) => {
                    out.total += n;
                    out.measured.insert(found.id, raw.len() as u64);
                }
                Measure::Over(cap) => {
                    let what = if is_objstm {
                        "object stream"
                    } else {
                        "cross-reference stream"
                    };
                    return Err(OpenError::Limit(format!(
                        "{what} {} {} {}",
                        found.id.0,
                        found.id.1,
                        cap_words(cap, limits)
                    )));
                }
                Measure::Stopped => return Err(OpenError::Limit("stopped by the caller".into())),
            }
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

/// Measures every stream of the loaded document that the raw scan did not, then refuses each
/// page that reaches a stream over a cap.
pub(crate) fn measure(
    doc: &Document,
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
    let mut bad: HashMap<ObjectId, String> = HashMap::new();
    let mut total_hit = false;
    for (&id, obj) in &doc.objects {
        let Object::Stream(stream) = obj else {
            continue;
        };
        // Skip what the raw scan charged, but only if it is the same stream: a dead object
        // with the same number must not stand in for a live one.
        if pre.measured.get(&id) == Some(&(stream.content.len() as u64))
            && (stream.dict.has_type(b"ObjStm") || stream.dict.has_type(b"XRef"))
        {
            continue;
        }
        if stop() {
            return Err(OpenError::Limit("stopped by the caller".into()));
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
        match measure_bytes(&stream.content, &filters, limits, budget, stop) {
            Measure::Fits(n) => verdict.unpacked += n,
            Measure::Over(cap) => {
                if cap == Cap::Total {
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
            Measure::Stopped => return Err(OpenError::Limit("stopped by the caller".into())),
        }
    }
    if bad.is_empty() {
        return Ok(verdict);
    }

    // Which pages reach a bad stream. The walk follows everything a page uses (contents,
    // resources, fonts, annotations and their appearances), but never the parent chain or
    // another page, so a link to a refused page does not refuse this one.
    let mut reached: HashSet<ObjectId> = HashSet::new();
    let mut steps = 0usize;
    for (index, &(page_id, inherited)) in pages.iter().enumerate() {
        let number = index as u32 + 1;
        let mut hit: Option<ObjectId> = None;
        let mut seen: HashSet<ObjectId> = HashSet::new();
        let mut todo: Vec<(&Object, usize)> = Vec::new();
        let page_obj = doc.objects.get(&page_id);
        seen.insert(page_id);
        if let Some(Object::Dictionary(d)) = page_obj {
            push_values(d, &mut todo, 1);
        }
        // Resources inherited from a page tree node above the page.
        if let Some(res) = inherited {
            todo.push((res, 1));
        }
        let mut exhausted = false;
        while let Some((obj, depth)) = todo.pop() {
            steps += 1;
            if steps > limits.max_items {
                exhausted = true;
                break;
            }
            if depth > limits.max_depth {
                continue;
            }
            let target = match obj {
                Object::Reference(id) => {
                    if !seen.insert(*id) {
                        continue;
                    }
                    if bad.contains_key(id) {
                        reached.insert(*id);
                        hit.get_or_insert(*id);
                    }
                    match doc.objects.get(id) {
                        Some(o) => o,
                        None => continue,
                    }
                }
                o => o,
            };
            match target {
                Object::Dictionary(d) => {
                    if is_page_node(d) {
                        continue;
                    }
                    push_values(d, &mut todo, depth + 1);
                }
                Object::Stream(s) => push_values(&s.dict, &mut todo, depth + 1),
                Object::Array(a) => todo.extend(a.iter().map(|o| (o, depth + 1))),
                _ => {}
            }
        }
        if exhausted {
            // Too much to walk: refuse what is left rather than guess.
            issues.push(Issue::new(
                "document",
                IssueKind::Limit,
                format!(
                    "the page resources need more than {} steps to check; pages from {number} on are refused",
                    limits.max_items
                ),
            ));
            verdict.refused_pages.extend(number..=pages.len() as u32);
            break;
        }
        if let Some(id) = hit {
            issues.push(Issue::new(
                &format!("page {number}"),
                IssueKind::Limit,
                format!(
                    "the page is not read: stream {} {} {}",
                    id.0, id.1, bad[&id]
                ),
            ));
            verdict.refused_pages.push(number);
        }
    }
    let mut rest: Vec<_> = bad.iter().filter(|(id, _)| !reached.contains(id)).collect();
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

fn push_values<'a>(d: &'a Dictionary, todo: &mut Vec<(&'a Object, usize)>, depth: usize) {
    for (k, v) in d.iter() {
        if matches!(k.as_slice(), b"Parent" | b"P") {
            continue;
        }
        todo.push((v, depth));
    }
}

fn is_page_node(d: &Dictionary) -> bool {
    matches!(d.get(b"Type"), Ok(Object::Name(n)) if n == b"Page" || n == b"Pages")
}

// ---- The raw scan ----

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
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

impl Scan<'_> {
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

struct RawStream {
    id: ObjectId,
    kind: Vec<u8>,
    filters: Vec<Filter>,
    /// Start and end of the stream data in the file.
    data: (usize, usize),
}

/// How far back from a `/Type` value the scan looks for the `obj` keyword.
const BACK: usize = 64 * 1024;

/// Reads the stream object around the name at `at`, if it is a stream whose `/Type` is
/// `ObjStm` or `XRef`.
fn raw_stream_at(bytes: &[u8], at: usize, limits: &Limits) -> Option<RawStream> {
    let lo = at.saturating_sub(BACK);
    let window = &bytes[lo..at];
    let mut pos = window.len();
    // The nearest `N G obj` before the name.
    let (id, dict_start) = loop {
        let k = window[..pos].windows(3).rposition(|w| w == b"obj")?;
        pos = k;
        let before = &window[..k];
        let after = window.get(k + 3).copied();
        if after.is_some_and(|c| !is_space(c) && !is_delimiter(c)) {
            continue;
        }
        if let Some(id) = object_header(before) {
            break (id, lo + k + 3);
        }
    };
    let mut scan = Scan {
        b: bytes,
        i: dict_start,
        items: 0,
        max_items: limits.max_items,
        max_depth: limits.max_depth,
    };
    let dict = scan.value(0)?;
    if !matches!(dict, Raw::Dict(_)) || scan.i <= at {
        return None;
    }
    let kind = match dict.get(b"Type") {
        Some(Raw::Name(n)) if n == b"ObjStm" || n == b"XRef" => n.clone(),
        _ => return None,
    };
    scan.skip();
    if bytes.get(scan.i..scan.i + 6) != Some(b"stream") {
        return None;
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
            let mut end = find(bytes, b"endstream", start)?;
            while end > start && matches!(bytes[end - 1], b'\r' | b'\n') {
                end -= 1;
            }
            end
        }
    };
    Some(RawStream {
        id,
        kind,
        filters: raw_filters(&dict),
        data: (start, end),
    })
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

    fn never() -> bool {
        false
    }

    fn zlib(data: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn chains_decode_to_the_right_length() {
        let limits = Limits::default();
        let m = |raw: &[u8], f: &[Filter]| measure_bytes(raw, f, &limits, u64::MAX, &never);
        assert_eq!(m(b"hello", &[]), Measure::Fits(5));
        assert_eq!(
            m(&zlib(b"hello world"), &[Filter::Flate]),
            Measure::Fits(11)
        );
        assert_eq!(m(b"68656c6c6f>", &[Filter::AsciiHex]), Measure::Fits(5));
        assert_eq!(m(b"6", &[Filter::AsciiHex]), Measure::Fits(1));
        assert_eq!(
            m(b"87cURD]i,\"Ebo80~>", &[Filter::Ascii85]),
            Measure::Fits(12)
        );
        assert_eq!(m(b"z~>", &[Filter::Ascii85]), Measure::Fits(4));
        // A literal of 3 bytes, then 4 copies of one byte, then the end.
        assert_eq!(
            m(&[2, b'a', b'b', b'c', 253, b'x', 128], &[Filter::RunLength]),
            Measure::Fits(7)
        );
        // Hex wrapped around deflate.
        let hex: String = zlib(b"abcdef").iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            m(hex.as_bytes(), &[Filter::AsciiHex, Filter::Flate]),
            Measure::Fits(6)
        );
        // An image codec ends the chain: its input is what is counted.
        assert_eq!(
            m(b"jpegdata", &[Filter::Opaque, Filter::Flate]),
            Measure::Fits(8)
        );
    }

    #[test]
    fn broken_zlib_header_is_retried_as_raw_deflate() {
        let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
        e.write_all(&[0u8; 100_000]).unwrap();
        let mut raw = vec![0xFF, 0xFF];
        raw.extend(e.finish().unwrap());
        let limits = Limits::default();
        assert_eq!(
            measure_bytes(&raw, &[Filter::Flate], &limits, u64::MAX, &never),
            Measure::Over(Cap::Ratio)
        );
    }

    #[test]
    fn caps_are_told_apart() {
        let packed = zlib(&[0u8; 200_000]);
        let small = Limits {
            max_stream_bytes: 1000,
            max_ratio: 1_000_000,
            ..Limits::default()
        };
        assert_eq!(
            measure_bytes(&packed, &[Filter::Flate], &small, u64::MAX, &never),
            Measure::Over(Cap::Stream)
        );
        let limits = Limits::default();
        assert_eq!(
            measure_bytes(&packed, &[Filter::Flate], &limits, u64::MAX, &never),
            Measure::Over(Cap::Ratio)
        );
        let roomy = Limits {
            max_ratio: 1_000_000,
            ..Limits::default()
        };
        assert_eq!(
            measure_bytes(&packed, &[Filter::Flate], &roomy, 10, &never),
            Measure::Over(Cap::Total)
        );
    }

    #[test]
    fn lzw_counts() {
        // "-----A---B" from ISO 32000-1, 7.4.4.2, with EarlyChange 1.
        let raw = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        let limits = Limits::default();
        assert_eq!(
            measure_bytes(
                &raw,
                &[Filter::Lzw { early_change: true }],
                &limits,
                u64::MAX,
                &never
            ),
            Measure::Fits(10)
        );
    }

    #[test]
    fn raw_scan_finds_object_streams() {
        let data = zlib(b"1 0 2 0 << >> << >>");
        let mut file =
            b"%PDF-1.5\n7 0 obj\n<< /Type /ObjStm /N 2 /First 8 /Filter /FlateDecode /Length "
                .to_vec();
        file.extend(format!("{} >>\nstream\n", data.len()).as_bytes());
        file.extend(&data);
        file.extend(b"\nendstream\nendobj\n");
        let pre = prescan(&file, &Limits::default(), &never).unwrap();
        assert_eq!(pre.measured.get(&(7, 0)), Some(&(data.len() as u64)));
        assert_eq!(pre.total, 19);
        assert_eq!(object_header(b"x\n12 0 "), Some((12, 0)));
        assert_eq!(object_header(b"end"), None);
        assert_eq!(object_header(b"a12 0 "), None);
    }
}
