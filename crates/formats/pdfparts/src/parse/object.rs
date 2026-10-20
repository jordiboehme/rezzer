// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One indirect object at a file position (ISO 32000-1, 7.3.10 and 7.3.8): its header, its
//! value and, for a stream, its raw data.

use std::collections::BTreeMap;

use lopdf::{Object, ObjectId, Stream};

use super::lexer::{Lexer, find, is_delimiter, is_space};
use super::{Budget, Fail};

/// An object read up to its value. For a stream, `stream` is where its data starts.
pub(crate) struct Head {
    pub id: ObjectId,
    pub value: Object,
    pub stream: Option<usize>,
}

/// Reads only the `N G obj` header at `pos`. White space and comments it skips count as work.
pub(crate) fn header_at(
    bytes: &[u8],
    pos: usize,
    budget: &mut Budget,
) -> Result<Option<ObjectId>, Fail> {
    let mut l = Lexer::new(bytes, pos);
    let id = l.header();
    budget.skipped(l.skipped)?;
    Ok(id)
}

/// The reason given for a broken object that was not read because the bound for broken
/// values was spent. Such objects share one issue.
pub(crate) const NOT_READ: &str = "not read: too many broken values";

/// How far past the first read a value that ran into its natural end is read again when no
/// next start is known. A broken value then costs at most its natural span plus this.
const RETRY_WINDOW: usize = 64 << 10;

/// The positions where the keyword `obj` starts, found once per stretch of the file.
#[derive(Default)]
pub(crate) struct Ends {
    /// Stretches searched so far: from a start to the next `obj` (or the end of the file),
    /// with no `obj` starting in between.
    spans: BTreeMap<usize, usize>,
    /// Where objects and cross-reference sections start, from the table or a scan, in order.
    starts: Vec<usize>,
}

impl Ends {
    /// Where the next `obj` at or after `p` starts, or the end of `bytes`. Every byte is
    /// searched about once, however often this is asked.
    pub fn next(&mut self, bytes: &[u8], p: usize) -> usize {
        let len = bytes.len();
        if p >= len {
            return len;
        }
        if let Some((_, &e)) = self.spans.range(..=p).next_back()
            && p <= e
        {
            return e;
        }
        let after = self.spans.range(p + 1..).next().map(|(&s, &e)| (s, e));
        let stop = after.map_or(len, |(s, _)| s);
        // A match may start right before `stop` and end after it.
        let hay = &bytes[..len.min(stop + 2)];
        let e = match find(hay, b"obj", p) {
            Some(q) => q,
            None => match after {
                Some((s, e)) => {
                    self.spans.remove(&s);
                    e
                }
                None => len,
            },
        };
        self.spans.insert(p, e);
        e
    }

    /// Adds known starts of objects and cross-reference sections.
    pub fn add_starts(&mut self, starts: impl IntoIterator<Item = usize>) {
        self.starts.extend(starts);
        self.starts.sort_unstable();
        self.starts.dedup();
    }

    /// The first known start after `p`.
    pub fn start_after(&self, p: usize) -> Option<usize> {
        let i = self.starts.partition_point(|&s| s <= p);
        self.starts.get(i).copied()
    }
}

/// Where a value should end, and how far it is read again when it runs into that end.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limit {
    /// Where the value should end: the next `obj`, member start or `trailer`.
    pub end: usize,
    /// `end` is the start of the next member of an object stream, so a value that reaches it
    /// is whole.
    pub member: bool,
    /// The next start of an object or member after `end`. A second read stops there; without
    /// it, [`RETRY_WINDOW`] past `end`.
    pub next: Option<usize>,
}

/// A value read up to `lim` that ends there is whole when the byte at `lim` ends any token or
/// when the value closes itself.
fn whole(bytes: &[u8], lim: usize) -> bool {
    let at = bytes.get(lim).copied();
    let value = &bytes[..lim];
    at.is_some_and(|c| is_space(c) || is_delimiter(c))
        || value.ends_with(b"]")
        || value.ends_with(b">>")
        || value.ends_with(b")")
}

/// Reads a value at `pos` with `read`, first only up to `limit.end`, where the value should
/// end. Returns the value and where it ends.
///
/// A broken value (a string that never closes) could otherwise read to the end of the file
/// each time, so the bytes it reads count against the bound for broken values, and what it
/// was charged as values is given back, so a value is charged once. A value that runs into
/// the end and is not whole there is read again without it, up to `limit.next` and while that
/// bound lasts, since a valid value may hold the bytes that the end stands for. The second
/// read never stops before the first one did.
/// Once the bound is spent, a value that fails or runs into the end is not read: it gives
/// [`NOT_READ`] and is counted in [`Budget::not_read`].
pub(crate) fn bounded<'b, T>(
    bytes: &'b [u8],
    pos: usize,
    limit: Limit,
    budget: &mut Budget,
    read: impl Fn(&mut Lexer<'b>, &mut Budget) -> Result<T, Fail>,
) -> Result<(T, usize), Fail> {
    let spent = budget.failed_left() == 0;
    let mut lim = limit.end.min(bytes.len());
    let mut again = false;
    loop {
        let mut l = Lexer::new(&bytes[..lim], pos);
        let (items, charged) = (budget.items, budget.bytes);
        let v = read(&mut l, budget);
        budget.skipped(l.skipped)?;
        let ran_in = !again && l.i >= lim && lim < bytes.len();
        let cut = ran_in && !(v.is_ok() && (limit.member || whole(bytes, lim)));
        let why = match v {
            Ok(v) if !cut => return Ok((v, l.i)),
            Err(Fail::Stop(e)) => return Err(Fail::Stop(e)),
            Ok(_) => None,
            Err(Fail::Bad(why)) => Some(why),
        };
        budget.items = items;
        budget.bytes = charged;
        budget.failed(l.i - pos);
        if cut && !spent && budget.failed_left() > 0 {
            let next = limit
                .next
                .filter(|&n| n > lim)
                .unwrap_or(lim.saturating_add(RETRY_WINDOW));
            let next = next
                .min(pos.saturating_add(budget.failed_left()))
                .min(bytes.len());
            if next > lim {
                again = true;
                lim = next;
                continue;
            }
        }
        return match why {
            Some(why) if !spent => Err(Fail::Bad(why)),
            _ => {
                budget.not_read += 1;
                Err(Fail::Bad(NOT_READ))
            }
        };
    }
}

/// Reads the header and the value of the object at `pos`, and finds out whether stream data
/// follows. White space and comments it skips count as work. The value should end before
/// the next `obj` (of `endobj` or of the next header), and is read again at most up to the
/// next known object start.
pub(crate) fn head_at(bytes: &[u8], pos: usize, budget: &mut Budget) -> Result<Head, Fail> {
    let mut l = Lexer::new(bytes, pos);
    let id = l.header();
    budget.skipped(l.skipped)?;
    let Some(id) = id else {
        return Err(Fail::Bad("there is no object header at its offset"));
    };
    let end = budget.ends.next(bytes, l.i);
    let limit = Limit {
        end,
        member: false,
        next: budget.ends.start_after(end),
    };
    let (value, at) = bounded(bytes, l.i, limit, budget, |l, b| l.value(b, 0))?;
    let stream = match value {
        Object::Dictionary(_) => {
            let mut l = Lexer::new(bytes, at);
            let start = l.stream_start();
            budget.skipped(l.skipped)?;
            start
        }
        _ => None,
    };
    Ok(Head { id, value, stream })
}

/// The `/Length` of a stream that is a reference.
pub(crate) fn length_ref(head: &Head) -> Option<ObjectId> {
    match (&head.value, head.stream) {
        (Object::Dictionary(d), Some(_)) => d.get(b"Length").ok()?.as_reference().ok(),
        _ => None,
    }
}

/// Finishes an object read by [`head_at`]. `length` is the value of an indirect `/Length`.
/// When the length is unknown or does not end at `endstream`, the data ends at the next
/// `endstream`. The `/Length` of a stream is always set to the length of its data.
pub(crate) fn finish(
    bytes: &[u8],
    head: Head,
    length: Option<i64>,
    budget: &mut Budget,
) -> Result<(ObjectId, Object), Fail> {
    let Some(start) = head.stream else {
        return Ok((head.id, head.value));
    };
    let Object::Dictionary(mut dict) = head.value else {
        return Err(Fail::Bad(
            "stream data follows a value that is not a dictionary",
        ));
    };
    let declared = match dict.get(b"Length") {
        Ok(Object::Integer(n)) => Some(*n),
        _ => length,
    };
    let end = declared
        .and_then(|n| usize::try_from(n).ok())
        .and_then(|n| start.checked_add(n))
        .filter(|&end| end <= bytes.len() && ends_stream(bytes, end));
    let end = match end {
        Some(end) => end,
        None => data_end(bytes, start, budget)?,
    };
    let content = &bytes[start..end];
    budget.value(content.len())?;
    dict.set("Length", content.len() as i64);
    Ok((
        head.id,
        Object::Stream(Stream {
            dict,
            content: content.to_vec(),
            allows_compression: false,
            start_position: None,
        }),
    ))
}

/// Reads the whole object at `pos`; `length` resolves an indirect `/Length`.
#[cfg(test)]
pub(crate) fn object_at(
    bytes: &[u8],
    pos: usize,
    budget: &mut Budget,
    length: &mut dyn FnMut(ObjectId) -> Option<i64>,
) -> Result<(ObjectId, Object), Fail> {
    let head = head_at(bytes, pos, budget)?;
    let len = length_ref(&head).and_then(length);
    finish(bytes, head, len, budget)
}

/// The keyword `endstream` follows `end`, after an optional line end.
fn ends_stream(bytes: &[u8], end: usize) -> bool {
    let mut l = Lexer::new(bytes, end);
    l.keyword(b"endstream")
}

/// The end of stream data found by looking for `endstream`, without the line end before it.
fn data_end(bytes: &[u8], start: usize, budget: &mut Budget) -> Result<usize, Fail> {
    let Some(at) = find(bytes, b"endstream", start) else {
        budget.work(bytes.len() - start)?;
        return Err(Fail::Bad("a stream has no end"));
    };
    budget.work(at - start)?;
    let mut end = at;
    if end > start && bytes[end - 1] == b'\n' {
        end -= 1;
    }
    if end > start && bytes[end - 1] == b'\r' {
        end -= 1;
    }
    Ok(end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Limits;

    fn read(file: &[u8]) -> Object {
        let limits = Limits::default();
        let mut budget = Budget::new(&limits, &|| false, file.len());
        object_at(file, 0, &mut budget, &mut |_| Some(3)).unwrap().1
    }

    #[test]
    fn stream_data_follows_its_length() {
        let s = read(b"4 0 obj << /Length 5 >>\nstream\r\nabcde\nendstream endobj");
        assert_eq!(s.as_stream().unwrap().content, b"abcde");
        // A wrong length is replaced by the data up to `endstream`.
        let s = read(b"4 0 obj << /Length 2 >> stream\nabcde\r\nendstream");
        assert_eq!(s.as_stream().unwrap().content, b"abcde");
        // An indirect length is resolved.
        let s = read(b"4 0 obj << /Length 9 0 R >> stream\nabc endstream");
        let s = s.as_stream().unwrap();
        assert_eq!(s.content, b"abc");
        assert_eq!(s.dict.get(b"Length").unwrap().as_i64().unwrap(), 3);
    }

    #[test]
    fn whole_values_at_the_limit_are_kept_when_the_bound_is_spent() {
        let limits = Limits::default();
        let file = b"<</A 1>>[1 2]/Xobj (a)2 0 obj";
        let mut budget = Budget::new(&limits, &|| false, file.len());
        budget.failed = budget.max_failed;
        let mut read = |pos: usize, end: usize, member: bool| {
            let limit = Limit {
                end,
                member,
                next: None,
            };
            bounded(file, pos, limit, &mut budget, |l, b| l.value(b, 0))
        };
        // Members that touch the next member start, and values that close themselves.
        assert!(read(0, 8, true).is_ok());
        assert!(read(8, 13, false).is_ok());
        assert!(read(19, 22, false).is_ok());
        // A name cut inside `Xobj` is not whole, and is not read again.
        assert_eq!(read(13, 16, false).unwrap_err(), Fail::Bad(NOT_READ));
        // A number at the limit is whole before white space.
        assert_eq!(read(22, 23, false).unwrap().1, 23);
        assert_eq!(budget.not_read, 1);
    }

    #[test]
    fn a_cut_value_is_charged_once() {
        let limits = Limits::default();
        let file = b"[(a obj) 1 2] 9 0 obj";
        let mut budget = Budget::new(&limits, &|| false, file.len());
        let limit = Limit {
            end: 5,
            member: false,
            next: Some(14),
        };
        let (v, at) = bounded(file, 0, limit, &mut budget, |l, b| l.value(b, 0)).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 3);
        assert_eq!(at, 13);
        // The array and its three items, once.
        assert_eq!(budget.items, 4);
    }

    #[test]
    fn next_obj_is_found_in_any_order() {
        let file = b"1 0 obj x endobj 2 0 obj y endobj trailer";
        let want = |p: usize| {
            file[p.min(file.len())..]
                .windows(3)
                .position(|w| w == b"obj")
                .map_or(file.len(), |q| q + p)
        };
        let mut ends = Ends::default();
        for p in [20, 30, 0, 9, 4, 5, 33, 40, 100, 21, 1] {
            assert_eq!(ends.next(file, p), want(p).min(file.len()), "{p}");
        }
    }
}
