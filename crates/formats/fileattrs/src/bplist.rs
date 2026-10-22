// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A reader for one shape of binary property list: `bplist00` with an array of strings on
//! top. Every number in the file is checked against the bytes before it is used, so a
//! hostile file gives an error, never a panic or a large allocation.

use std::fmt;

/// The most strings [`string_array`] returns.
pub const MAX_ITEMS: usize = 1024;

/// The most UTF-8 bytes of text [`string_array`] returns in total.
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;

/// The deepest nesting of arrays, sets and dictionaries, the top array included.
pub const MAX_DEPTH: usize = 8;

/// The most objects visited while checking nested containers for cycles.
pub const MAX_VISITS: usize = 16 * 1024;

const HEADER: &[u8; 8] = b"bplist00";
const TRAILER_LEN: usize = 32;

/// Why a binary property list could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BplistError {
    /// Shorter than a header and a trailer.
    TooShort,
    /// The header is not `bplist00`.
    NotBplist00,
    /// The trailer holds an impossible size, count or position.
    BadTrailer,
    /// An object position is outside the object area.
    BadOffset,
    /// A reference names an object that does not exist.
    BadReference,
    /// A length or count runs past the object area, or is not a valid integer.
    BadLength,
    /// The top object is not an array.
    NotAnArray,
    /// A container holds itself, directly or through other containers.
    Cycle,
    /// Containers nest deeper than [`MAX_DEPTH`].
    TooDeep,
    /// Checking nested containers would visit more than [`MAX_VISITS`] objects.
    TooManyObjects,
    /// The top array has more than [`MAX_ITEMS`] entries.
    TooManyItems,
    /// The strings hold more than [`MAX_TEXT_BYTES`] of text.
    TooMuchText,
}

impl fmt::Display for BplistError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::TooShort => "too short for a binary property list",
            Self::NotBplist00 => "not a readable property list (header is not bplist00)",
            Self::BadTrailer => "the trailer holds impossible sizes or positions",
            Self::BadOffset => "an object position is outside the object area",
            Self::BadReference => "a reference names an object that does not exist",
            Self::BadLength => "a length runs past the object area or is not valid",
            Self::NotAnArray => "the top object is not an array",
            Self::Cycle => "a container holds itself",
            Self::TooDeep => "containers nest too deep",
            Self::TooManyObjects => "too many nested objects",
            Self::TooManyItems => "the array has too many entries",
            Self::TooMuchText => "the strings hold too much text",
        };
        f.write_str(text)
    }
}

impl std::error::Error for BplistError {}

/// Reads a `bplist00` whose top object is an array of strings.
///
/// ASCII, UTF-16 and UTF-8 string objects are returned in array order, unchanged. An entry
/// that is not a string, or a string whose bytes are not valid text, is skipped. Nested
/// containers are skipped too, but they are still checked for cycles and depth.
///
/// # Errors
///
/// A broken layout, a cycle, nesting deeper than [`MAX_DEPTH`], more than [`MAX_ITEMS`]
/// entries or more than [`MAX_TEXT_BYTES`] of text.
pub fn string_array(bytes: &[u8]) -> Result<Vec<String>, BplistError> {
    let plist = Plist::parse(bytes)?;
    let top = plist.object(plist.top)?;
    let (kind, refs) = match plist.container(top)? {
        Some(found) => found,
        None => return Err(BplistError::NotAnArray),
    };
    if kind != 0xA {
        return Err(BplistError::NotAnArray);
    }
    if refs.count > MAX_ITEMS {
        return Err(BplistError::TooManyItems);
    }

    let mut walk = Walk {
        path: vec![plist.top],
        visits: 1,
    };
    let mut out = Vec::with_capacity(refs.count);
    let mut text = 0usize;
    for i in 0..refs.count {
        let index = plist.reference(refs, i)?;
        let at = plist.object(index)?;
        if let Some(string) = plist.string(at)? {
            text = text.saturating_add(string.len());
            if text > MAX_TEXT_BYTES {
                return Err(BplistError::TooMuchText);
            }
            out.push(string);
        } else {
            walk.check(&plist, index)?;
        }
    }
    Ok(out)
}

/// The checked trailer values and the bytes they describe.
struct Plist<'a> {
    bytes: &'a [u8],
    /// End of the object area, the start of the offset table.
    area_end: usize,
    offset_size: usize,
    ref_size: usize,
    count: usize,
    top: usize,
}

/// The references of one container: where they start and how many there are.
#[derive(Clone, Copy)]
struct Refs {
    start: usize,
    count: usize,
}

impl<'a> Plist<'a> {
    fn parse(bytes: &'a [u8]) -> Result<Self, BplistError> {
        if bytes.len() < HEADER.len() + TRAILER_LEN {
            return Err(BplistError::TooShort);
        }
        if &bytes[..HEADER.len()] != HEADER {
            return Err(BplistError::NotBplist00);
        }
        let trailer_start = bytes.len() - TRAILER_LEN;
        let trailer = &bytes[trailer_start..];
        let offset_size = usize::from(trailer[6]);
        let ref_size = usize::from(trailer[7]);
        if !(1..=8).contains(&offset_size) || !(1..=8).contains(&ref_size) {
            return Err(BplistError::BadTrailer);
        }
        let count = be_usize(&trailer[8..16]).ok_or(BplistError::BadTrailer)?;
        let top = be_usize(&trailer[16..24]).ok_or(BplistError::BadTrailer)?;
        let area_end = be_usize(&trailer[24..32]).ok_or(BplistError::BadTrailer)?;

        // Each object takes at least one byte of the object area.
        if area_end < HEADER.len() || area_end > trailer_start {
            return Err(BplistError::BadTrailer);
        }
        if count == 0 || count > area_end - HEADER.len() {
            return Err(BplistError::BadTrailer);
        }
        let table_len = count
            .checked_mul(offset_size)
            .ok_or(BplistError::BadTrailer)?;
        if table_len > trailer_start - area_end {
            return Err(BplistError::BadTrailer);
        }
        if top >= count {
            return Err(BplistError::BadTrailer);
        }
        Ok(Self {
            bytes,
            area_end,
            offset_size,
            ref_size,
            count,
            top,
        })
    }

    /// The position of object `index`, checked to lie in the object area.
    fn object(&self, index: usize) -> Result<usize, BplistError> {
        if index >= self.count {
            return Err(BplistError::BadReference);
        }
        // The table fits before the trailer (checked in parse).
        let entry = self.area_end + index * self.offset_size;
        let at =
            be_usize(&self.bytes[entry..entry + self.offset_size]).ok_or(BplistError::BadOffset)?;
        if at < HEADER.len() || at >= self.area_end {
            return Err(BplistError::BadOffset);
        }
        Ok(at)
    }

    /// Reference `i` of a container, as an object index.
    fn reference(&self, refs: Refs, i: usize) -> Result<usize, BplistError> {
        // The references fit in the object area (checked in body).
        let at = refs.start + i * self.ref_size;
        let index =
            be_usize(&self.bytes[at..at + self.ref_size]).ok_or(BplistError::BadReference)?;
        if index >= self.count {
            return Err(BplistError::BadReference);
        }
        Ok(index)
    }

    /// The marker type nibble, and the start and length of the body after the marker and
    /// any length integer. `unit` is the size of one counted element; the body is checked
    /// to end inside the object area.
    fn body(&self, at: usize, unit: usize) -> Result<(u8, usize, usize), BplistError> {
        let marker = self.bytes[at];
        let nibble = marker & 0x0F;
        let mut start = at + 1;
        let count = if nibble == 0x0F {
            let (value, next) = self.length_integer(start)?;
            start = next;
            value
        } else {
            usize::from(nibble)
        };
        let len = count.checked_mul(unit).ok_or(BplistError::BadLength)?;
        let end = start.checked_add(len).ok_or(BplistError::BadLength)?;
        if end > self.area_end {
            return Err(BplistError::BadLength);
        }
        Ok((marker >> 4, start, count))
    }

    /// An integer object holding a length, at `at`. Returns the value and the position
    /// after it.
    fn length_integer(&self, at: usize) -> Result<(usize, usize), BplistError> {
        if at >= self.area_end {
            return Err(BplistError::BadLength);
        }
        let marker = self.bytes[at];
        if marker >> 4 != 0x1 {
            return Err(BplistError::BadLength);
        }
        let width = match marker & 0x0F {
            power @ 0..=4 => 1usize << power,
            _ => return Err(BplistError::BadLength),
        };
        let start = at + 1;
        let end = start.checked_add(width).ok_or(BplistError::BadLength)?;
        if end > self.area_end {
            return Err(BplistError::BadLength);
        }
        let digits = &self.bytes[start..end];
        let value = match width {
            // Eight bytes are a signed value: the top bit means negative.
            8 if digits[0] & 0x80 != 0 => return Err(BplistError::BadLength),
            // Sixteen bytes: only values that fit in 64 bits.
            16 if digits[..8].iter().any(|&b| b != 0) => return Err(BplistError::BadLength),
            16 => be_usize(&digits[8..]),
            _ => be_usize(digits),
        };
        Ok((value.ok_or(BplistError::BadLength)?, end))
    }

    /// The text of a string object, `Ok(None)` when the object is no string or its bytes
    /// are not valid text.
    fn string(&self, at: usize) -> Result<Option<String>, BplistError> {
        match self.bytes[at] >> 4 {
            0x5 => {
                let (_, start, len) = self.body(at, 1)?;
                let raw = &self.bytes[start..start + len];
                // ASCII is valid UTF-8, so the conversion keeps every byte.
                Ok(raw
                    .is_ascii()
                    .then(|| String::from_utf8_lossy(raw).into_owned()))
            }
            0x6 => {
                let (_, start, units) = self.body(at, 2)?;
                let raw = &self.bytes[start..start + units * 2];
                let units = raw
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|p| u16::from_be_bytes(*p));
                Ok(char::decode_utf16(units)
                    .collect::<Result<String, _>>()
                    .ok())
            }
            0x7 => {
                let (_, start, len) = self.body(at, 1)?;
                Ok(std::str::from_utf8(&self.bytes[start..start + len])
                    .ok()
                    .map(str::to_owned))
            }
            _ => Ok(None),
        }
    }

    /// The type nibble and references of an array, set or dictionary at `at`, or `None`
    /// for any other object. A dictionary has twice as many references as entries.
    fn container(&self, at: usize) -> Result<Option<(u8, Refs)>, BplistError> {
        let kind = self.bytes[at] >> 4;
        let per_entry = match kind {
            0xA | 0xC => 1,
            0xD => 2,
            _ => return Ok(None),
        };
        let unit = self
            .ref_size
            .checked_mul(per_entry)
            .ok_or(BplistError::BadLength)?;
        let (_, start, entries) = self.body(at, unit)?;
        let count = entries * per_entry;
        Ok(Some((kind, Refs { start, count })))
    }
}

/// The containers on the way down from the top array, and the visits so far.
struct Walk {
    path: Vec<usize>,
    visits: usize,
}

impl Walk {
    /// Checks object `index` and everything it holds for cycles, depth and the visit
    /// budget. Objects that are no containers need no check.
    fn check(&mut self, plist: &Plist<'_>, index: usize) -> Result<(), BplistError> {
        self.visits += 1;
        if self.visits > MAX_VISITS {
            return Err(BplistError::TooManyObjects);
        }
        if self.path.contains(&index) {
            return Err(BplistError::Cycle);
        }
        let at = plist.object(index)?;
        let Some((_, refs)) = plist.container(at)? else {
            return Ok(());
        };
        if self.path.len() >= MAX_DEPTH {
            return Err(BplistError::TooDeep);
        }
        self.path.push(index);
        for i in 0..refs.count {
            let child = plist.reference(refs, i)?;
            self.check(plist, child)?;
        }
        self.path.pop();
        Ok(())
    }
}

/// A big-endian unsigned integer of up to eight bytes, `None` when it does not fit `usize`.
fn be_usize(bytes: &[u8]) -> Option<usize> {
    let mut value: u64 = 0;
    for &b in bytes {
        value = value.checked_mul(256)?.checked_add(u64::from(b))?;
    }
    usize::try_from(value).ok()
}
