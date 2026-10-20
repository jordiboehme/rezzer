// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One indirect object at a file position (ISO 32000-1, 7.3.10 and 7.3.8): its header, its
//! value and, for a stream, its raw data.

use lopdf::{Object, ObjectId, Stream};

use super::lexer::{Lexer, find};
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

/// Reads the header and the value of the object at `pos`, and finds out whether stream data
/// follows. White space and comments it skips count as work.
pub(crate) fn head_at(bytes: &[u8], pos: usize, budget: &mut Budget) -> Result<Head, Fail> {
    let mut l = Lexer::new(bytes, pos);
    let Some(id) = l.header() else {
        budget.skipped(l.skipped)?;
        return Err(Fail::Bad("there is no object header at its offset"));
    };
    let value = l.value(budget, 0);
    let stream = match value {
        Ok(Object::Dictionary(_)) => l.stream_start(),
        _ => None,
    };
    budget.skipped(l.skipped)?;
    if value.is_err() {
        budget.failed(l.i - pos)?;
    }
    Ok(Head {
        id,
        value: value?,
        stream,
    })
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
}
