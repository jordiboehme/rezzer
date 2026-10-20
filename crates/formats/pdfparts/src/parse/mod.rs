// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The only code that parses the file. It reads the cross-reference chain (or rebuilds it),
//! copies the objects reachable from the trailer into a lopdf document in memory, decrypts
//! them, and writes that document as canonical bytes: a classic cross-reference table, no
//! object streams, no comments, no unreachable objects, no encryption. Other parsers only ever
//! see those bytes.
//!
//! Every value is charged a fixed [`VALUE_COST`] plus the bytes of its strings, names and
//! stream data against `max_object_bytes`. `max_items` bounds one structure: the items of
//! one array or dictionary, or the members listed in one object stream.

mod crypt;
mod lexer;
mod object;
mod predict;
mod store;
mod xref;

use std::collections::HashMap;
use std::io::{self, Write};

use lopdf::xref::XrefType;
use lopdf::{Dictionary, Document, Object};

use crate::{Encryption, Issue, IssueKind, Limits, OpenError};
use lexer::Lexer;
use store::Store;
use xref::{Entry, read_chain, rebuild};

/// What every parsed value costs, on top of its bytes. About the size of a lopdf object.
pub(crate) const VALUE_COST: u64 = 128;

/// What one cross-reference entry costs.
const ENTRY_COST: u64 = 32;

/// How many times the file length (and the length of unpacked object streams) the searches
/// for `endstream` and skipped white space may read in total. Broken values have a bound of
/// the same size of their own.
const WORK_FACTOR: usize = 8;

/// How often, in charged values, the stop callback is asked.
const POLL_EVERY: usize = 4096;

pub(crate) fn stopped() -> OpenError {
    OpenError::Limit("stopped by the caller".into())
}

/// Why a value or an object could not be read.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Fail {
    /// This piece is damaged; the rest of the file may still be read.
    Bad(&'static str),
    /// A limit was hit or the caller asked to stop: the open ends.
    Stop(OpenError),
}

impl Fail {
    pub fn into_open(self) -> OpenError {
        match self {
            Fail::Bad(why) => OpenError::Damaged(why.into()),
            Fail::Stop(e) => e,
        }
    }
}

/// What reading the file may cost.
pub(crate) struct Budget<'a> {
    pub limits: &'a Limits,
    pub max_depth: usize,
    pub stop: &'a dyn Fn() -> bool,
    pub items: usize,
    pub bytes: u64,
    /// Bytes object streams and cross-reference streams unpacked to.
    pub unpacked: u64,
    work: usize,
    max_work: usize,
    /// Bytes read by values that turned out broken, and their bound.
    failed: usize,
    max_failed: usize,
    /// Broken objects that were not read because the bound for broken values was spent.
    pub not_read: usize,
    /// Where the next `obj` keyword of the file starts. Only for the bytes of the file, never
    /// for unpacked data.
    pub ends: object::Ends,
    next_poll: usize,
}

impl<'a> Budget<'a> {
    pub fn new(limits: &'a Limits, stop: &'a dyn Fn() -> bool, file_len: usize) -> Budget<'a> {
        Budget {
            limits,
            max_depth: limits.max_depth,
            stop,
            items: 0,
            bytes: 0,
            unpacked: 0,
            work: 0,
            max_work: file_len.saturating_mul(WORK_FACTOR).saturating_add(1 << 20),
            failed: 0,
            max_failed: file_len.saturating_mul(WORK_FACTOR).saturating_add(1 << 20),
            not_read: 0,
            ends: object::Ends::default(),
            next_poll: POLL_EVERY,
        }
    }

    fn add(&mut self, cost: u64) -> Result<(), Fail> {
        // Values are counted for the record only: their memory is bounded by
        // `max_object_bytes`, so the number of pages or fields has no cap of its own.
        self.items += 1;
        self.bytes = self.bytes.saturating_add(cost);
        self.check()?;
        if self.items >= self.next_poll {
            self.next_poll += POLL_EVERY;
            if (self.stop)() {
                return Err(Fail::Stop(stopped()));
            }
        }
        Ok(())
    }

    fn check(&self) -> Result<(), Fail> {
        if self.bytes > self.limits.max_object_bytes {
            return Err(Fail::Stop(OpenError::Limit(format!(
                "the objects of the file take more than {} bytes",
                self.limits.max_object_bytes
            ))));
        }
        Ok(())
    }

    /// Charges one value with `payload` bytes of strings, names or stream data.
    pub fn value(&mut self, payload: usize) -> Result<(), Fail> {
        self.add(VALUE_COST.saturating_add(payload as u64))
    }

    /// Charges one cross-reference entry.
    pub fn entry(&mut self) -> Result<(), Fail> {
        self.add(ENTRY_COST)
    }

    /// Charges bytes that are not a value.
    pub fn charge(&mut self, n: u64) -> Result<(), Fail> {
        self.bytes = self.bytes.saturating_add(n);
        self.check()
    }

    /// Counts bytes read while looking for the end of a stream.
    pub fn work(&mut self, n: usize) -> Result<(), Fail> {
        self.count_work(n, "its streams have no usable length")
    }

    /// Counts white space and comments skipped. They share the bound with [`Budget::work`].
    pub fn skipped(&mut self, n: usize) -> Result<(), Fail> {
        self.count_work(n, "it has too much white space or too many comments")
    }

    /// Counts bytes read by a value that turned out broken. A broken value is not charged
    /// as a value, so this bounds how often the same bytes are read in vain. It never ends
    /// the open: once it is spent, broken values are no longer read again (see
    /// [`object::bounded`]).
    pub fn failed(&mut self, n: usize) {
        self.failed = self.failed.saturating_add(n);
    }

    /// What broken values may still read.
    pub fn failed_left(&self) -> usize {
        self.max_failed.saturating_sub(self.failed)
    }

    fn count_work(&mut self, n: usize, why: &str) -> Result<(), Fail> {
        self.work = self.work.saturating_add(n);
        if self.work > self.max_work {
            return Err(Fail::Stop(OpenError::Limit(format!(
                "the file is too irregular: {why}"
            ))));
        }
        Ok(())
    }

    /// Raises the work bound for `n` more bytes to read, as unpacked streams add them.
    pub fn more_work(&mut self, n: u64) {
        let n = usize::try_from(n).unwrap_or(usize::MAX);
        self.max_work = self.max_work.saturating_add(n.saturating_mul(WORK_FACTOR));
        self.max_failed = self
            .max_failed
            .saturating_add(n.saturating_mul(WORK_FACTOR));
    }

    /// The work or byte bound was hit.
    pub fn spent(&self) -> bool {
        self.work > self.max_work || self.bytes > self.limits.max_object_bytes
    }

    /// Asks the stop callback.
    pub fn poll(&mut self) -> Result<(), Fail> {
        if (self.stop)() {
            return Err(Fail::Stop(stopped()));
        }
        Ok(())
    }
}

/// The document as other parsers may see it.
pub(crate) struct Built {
    pub doc: Document,
    pub canonical: Vec<u8>,
    pub encryption: Option<Encryption>,
    pub issues: Vec<Issue>,
    /// Bytes object streams and cross-reference streams unpacked to.
    pub unpacked: u64,
    /// Bytes charged for the parsed objects and the canonical copy.
    pub charged: u64,
    /// Values parsed.
    pub items: usize,
}

/// How far into the file the `%PDF-` header may start.
pub(crate) const HEADER_WINDOW: usize = 1024;

/// The version in the header, if it looks like one.
fn version(bytes: &[u8], base: usize) -> String {
    let v: Vec<u8> = bytes[base + 5..]
        .iter()
        .take(8)
        .take_while(|c| c.is_ascii_digit() || **c == b'.')
        .copied()
        .collect();
    let ok = v.len() >= 3 && v[0].is_ascii_digit() && v.iter().filter(|c| **c == b'.').count() == 1;
    if ok {
        String::from_utf8_lossy(&v).into_owned()
    } else {
        "1.7".to_string()
    }
}

/// Reads the file into a document and its canonical bytes.
pub(crate) fn build(
    bytes: &[u8],
    password: Option<&str>,
    limits: &Limits,
    stop: &dyn Fn() -> bool,
) -> Result<Built, OpenError> {
    let head = &bytes[..bytes.len().min(HEADER_WINDOW)];
    let Some(base) = head.windows(5).position(|w| w == b"%PDF-") else {
        return Err(OpenError::NotPdf);
    };
    let mut budget = Budget::new(limits, stop, bytes.len());
    let mut issues = Vec::new();
    let (table, trailer, scanned) = match read_chain(bytes, base, &mut budget)? {
        Some(chain) => {
            budget.ends.add_starts(chain.starts());
            issues.extend(chain.issues);
            (chain.table, chain.trailer, None)
        }
        None => {
            let found = rebuild(bytes, &mut budget)?;
            budget.ends.add_starts(found.starts());
            let trailer = scan_trailer(bytes, &found, &mut budget)?;
            let table: HashMap<u32, Entry> = found
                .table
                .iter()
                .map(|(&n, &(pos, generation))| (n, Entry::Normal { pos, generation }))
                .collect();
            (table, trailer, Some(found.table))
        }
    };
    let mut store = Store::new(bytes, base, table, budget, scanned);
    let Ok(Object::Reference(root)) = trailer.get(b"Root") else {
        return Err(OpenError::Damaged(
            "the file has no document catalog".into(),
        ));
    };
    store.root = Some(*root);
    let mut encryption = None;
    if let Some((crypt, enc)) = crypt::setup(&mut store, &trailer, password)? {
        store.crypt = Some(crypt);
        encryption = Some(enc);
    }
    let mut roots: Vec<&Object> = Vec::new();
    for key in [b"Root".as_slice(), b"Info"] {
        if let Ok(v) = trailer.get(key) {
            roots.push(v);
        }
    }
    let mut objects = store.reach(&roots)?;
    // lopdf's writer leaves out objects of these types. The document keeps exactly what is
    // written.
    objects.retain(|_, o| !matches!(o.type_name(), Ok(b"ObjStm" | b"XRef" | b"Linearized")));
    issues.append(&mut store.issues);
    let mut budget = store.budget;
    if budget.not_read > 0 {
        let n = budget.not_read;
        let what = if n == 1 { "object" } else { "objects" };
        issues.push(Issue::new(
            "document",
            IssueKind::Damaged,
            format!("{n} {what} not read: too many broken values"),
        ));
    }

    let mut doc = Document::with_version(version(bytes, base));
    doc.reference_table.cross_reference_type = XrefType::CrossReferenceTable;
    doc.max_id = objects.keys().map(|id| id.0).max().unwrap_or(0);
    doc.objects = objects;
    let mut canon_trailer = Dictionary::new();
    for key in [b"Root".as_slice(), b"Info", b"ID"] {
        if let Ok(v) = trailer.get(key) {
            canon_trailer.set(key, v.clone());
        }
    }
    doc.trailer = canon_trailer;
    let left = limits.max_object_bytes.saturating_sub(budget.bytes);
    let mut out = Capped {
        out: Vec::new(),
        left,
    };
    if doc.save_to(&mut out).is_err() {
        return Err(OpenError::Limit(format!(
            "the objects of the file take more than {} bytes",
            limits.max_object_bytes
        )));
    }
    budget
        .charge(out.out.len() as u64)
        .map_err(Fail::into_open)?;
    Ok(Built {
        doc,
        canonical: out.out,
        encryption,
        issues,
        unpacked: budget.unpacked,
        charged: budget.bytes,
        items: budget.items,
    })
}

/// A writer that fails once it has taken `left` bytes.
struct Capped {
    out: Vec<u8>,
    left: u64,
}

impl Write for Capped {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = buf.len() as u64;
        if n > self.left {
            return Err(io::Error::other("over the object budget"));
        }
        self.left -= n;
        self.out.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// How many bytes per byte of the file the look ahead for the trailer and the catalog may
/// charge in total: a value costs [`VALUE_COST`] and takes at least one byte and a separator.
const LOOK_FACTOR: u64 = VALUE_COST;

/// The trailer of a file whose table was rebuilt: the newest `trailer` dictionary, else the
/// newest cross-reference stream dictionary, else the newest catalog.
///
/// This is a look ahead: the values are dropped right away, and objects are read again (and
/// charged) only if the document uses them. All of it shares one work and byte bound that
/// grows with the file length, so pieces that overlap (a string that never closes swallows
/// the rest of the file) cannot make it quadratic.
fn scan_trailer(
    bytes: &[u8],
    found: &xref::Rebuilt,
    budget: &mut Budget,
) -> Result<Dictionary, OpenError> {
    let len = bytes.len() as u64;
    let look_limits = Limits {
        max_object_bytes: budget
            .limits
            .max_object_bytes
            .min(len.saturating_mul(LOOK_FACTOR).saturating_add(1 << 20)),
        ..*budget.limits
    };
    let mut look = Budget::new(&look_limits, budget.stop, bytes.len());
    look.ends.add_starts(found.starts());
    let spent = |look: &Budget, e: OpenError| {
        if look.spent() {
            OpenError::Limit(
                "the cross-reference table is damaged and the file is too irregular to find its catalog"
                    .into(),
            )
        } else {
            e
        }
    };
    let mut trailer = Dictionary::new();
    let merge = |trailer: &mut Dictionary, d: &Dictionary| {
        for key in [b"Root".as_slice(), b"Info", b"Encrypt", b"ID"] {
            if trailer.get(key).is_err()
                && let Ok(v) = d.get(key)
            {
                trailer.set(key, v.clone());
            }
        }
    };
    // A trailer dictionary ends before the next `trailer` keyword.
    let mut starts = found.trailers.clone();
    starts.sort_unstable();
    for (k, &at) in found.trailers.iter().rev().enumerate() {
        if k % 1024 == 0 {
            look.poll().map_err(Fail::into_open)?;
        }
        let mut l = Lexer::new(bytes, at);
        l.skip();
        if let Err(Fail::Stop(e)) = look.skipped(l.skipped) {
            return Err(spent(&look, e));
        }
        if !bytes[l.i..].starts_with(b"<<") {
            continue;
        }
        let end = match starts.binary_search(&l.i) {
            Ok(i) | Err(i) => starts.get(i).copied().unwrap_or(bytes.len()),
        };
        let limit = object::Limit {
            end,
            member: false,
            next: None,
        };
        let d = object::bounded(bytes, l.i, limit, &mut look, |l, b| {
            l.i += 2;
            l.dictionary(b, 0)
        });
        match d {
            Ok((d, _)) => merge(&mut trailer, &d),
            Err(Fail::Bad(_)) => {}
            Err(Fail::Stop(e)) => return Err(spent(&look, e)),
        }
    }
    if trailer.get(b"Root").is_ok() {
        return Ok(trailer);
    }
    let mut by_pos: Vec<(usize, u32, u16)> = found
        .table
        .iter()
        .map(|(&n, &(pos, g))| (pos, n, g))
        .collect();
    by_pos.sort_unstable_by(|a, b| b.cmp(a));
    for (k, (pos, n, g)) in by_pos.into_iter().enumerate() {
        if k % 1024 == 0 {
            look.poll().map_err(Fail::into_open)?;
        }
        // Only a dictionary can be a catalog or a cross-reference stream, so nothing else is
        // parsed.
        let mut l = Lexer::new(bytes, pos);
        let header = l.header();
        l.skip();
        if let Err(Fail::Stop(e)) = look.skipped(l.skipped) {
            return Err(spent(&look, e));
        }
        if header.is_none() || !bytes[l.i..].starts_with(b"<<") {
            continue;
        }
        let head = match object::head_at(bytes, pos, &mut look) {
            Ok(h) => h,
            Err(Fail::Bad(_)) => continue,
            Err(Fail::Stop(e)) => return Err(spent(&look, e)),
        };
        let Object::Dictionary(d) = &head.value else {
            continue;
        };
        if d.has_type(b"XRef") {
            merge(&mut trailer, d);
        } else if d.has_type(b"Catalog") && trailer.get(b"Root").is_err() {
            trailer.set("Root", Object::Reference((n, g)));
        }
        if trailer.get(b"Root").is_ok() {
            break;
        }
    }
    Ok(trailer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_cost_covers_an_object() {
        let size = std::mem::size_of::<Object>() as u64;
        eprintln!("size of a lopdf object: {size} bytes");
        assert!(size <= VALUE_COST, "{size}");
    }
}
