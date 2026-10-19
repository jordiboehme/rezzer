// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A bounded walk over a ZIP central directory, for detection only.
//!
//! Memory stays constant: headers are read one by one from the byte slice and only the
//! entries named in a marker list are kept. At most [`MAX_ENTRIES`] headers are read. Entry
//! data is read from the local header, stored or deflated, within a byte budget.

use std::io::Read;

/// Most central directory headers read during detection.
pub(crate) const MAX_ENTRIES: u64 = 100_000;
/// Largest central directory accepted during detection.
pub(crate) const MAX_DIRECTORY_BYTES: u64 = 64 * 1024 * 1024;

const EOCD: &[u8] = b"PK\x05\x06";
const EOCD64: &[u8] = b"PK\x06\x06";
const LOCATOR: &[u8] = b"PK\x06\x07";
const CENTRAL: &[u8] = b"PK\x01\x02";
const LOCAL: &[u8] = b"PK\x03\x04";
/// How far before the ZIP64 locator the ZIP64 end record is searched.
const EOCD64_SEARCH: usize = 64 * 1024;
/// Most deflate input read for one entry. Real data that inflates to the 64 KiB budget is
/// far smaller; only padding such as empty blocks gets this long.
const MAX_INFLATE_INPUT: usize = 1024 * 1024;

/// One entry of interest, as found in the central directory.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Entry {
    flags: u16,
    method: u16,
    compressed: u64,
    uncompressed: u64,
    /// Offset of the local header, already corrected by the archive shift.
    local: u64,
}

/// The result of [`walk`].
pub(crate) enum Walk {
    TooLarge(String),
    Unreadable(String),
    Listed(Listing),
}

pub(crate) struct Listing {
    /// Headers read.
    pub entries: u64,
    /// One slot per marker, in marker order.
    pub markers: Vec<Option<Entry>>,
    /// Some entry path contains `.BlobParts/` (Fusion 360).
    pub blob_parts: bool,
}

/// The central directory as located from an end record.
struct Directory {
    /// Absolute start of the directory in `bytes`.
    start: u64,
    size: u64,
    /// Entry count, None when the 16-bit field is saturated and no ZIP64 record exists.
    count: Option<u64>,
    /// Bytes in front of the archive (self-extracting stubs, junk); added to every offset.
    shift: u64,
}

/// True when a consistent end record with at least one entry sits in the tail of `bytes`,
/// for archives behind a stub (self-extracting archives).
pub(crate) fn has_directory(bytes: &[u8]) -> bool {
    locate(bytes).is_ok_and(|d| d.size > 0 && d.count != Some(0))
}

/// Walks the central directory. Markers are compared without regard to ASCII case. The walk
/// stops early once the marker at `stop_marker` is found.
pub(crate) fn walk(bytes: &[u8], markers: &[&str], stop_marker: Option<usize>) -> Walk {
    let dir = match locate(bytes) {
        Ok(d) => d,
        Err(why) => return Walk::Unreadable(why),
    };
    if let Some(n) = dir.count
        && n > MAX_ENTRIES
    {
        return Walk::TooLarge(format!("{n} entries, more than {MAX_ENTRIES}"));
    }
    if dir.size > MAX_DIRECTORY_BYTES {
        return Walk::TooLarge(format!(
            "{} bytes of directory, more than {MAX_DIRECTORY_BYTES}",
            dir.size
        ));
    }
    let mut listing = Listing {
        entries: 0,
        markers: vec![None; markers.len()],
        blob_parts: false,
    };
    let end = dir.start + dir.size;
    let mut p = dir.start;
    loop {
        match dir.count {
            Some(n) if listing.entries == n => break,
            Some(_) if p >= end => {
                return Walk::Unreadable(format!(
                    "the directory ends before entry {}",
                    listing.entries + 1
                ));
            }
            None if p >= end => break,
            _ => {}
        }
        if listing.entries == MAX_ENTRIES {
            return Walk::TooLarge(format!("more than {MAX_ENTRIES} entries"));
        }
        let Some((entry, name, next)) = central_header(bytes, p, dir.shift) else {
            return Walk::Unreadable(format!(
                "central directory header {} is broken",
                listing.entries + 1
            ));
        };
        listing.entries += 1;
        if let Some(i) = markers
            .iter()
            .position(|m| m.as_bytes().eq_ignore_ascii_case(name))
            && listing.markers[i].is_none()
        {
            listing.markers[i] = Some(entry);
            if stop_marker == Some(i) {
                return Walk::Listed(listing);
            }
        } else if !listing.blob_parts {
            listing.blob_parts = name
                .windows(11)
                .any(|w| w.eq_ignore_ascii_case(b".BlobParts/"));
        }
        p = next;
    }
    Walk::Listed(listing)
}

/// Finds the end record. Every `PK\x05\x06` in the last 64 KiB + 22 bytes is tried, from the
/// end backwards, and the first one whose directory fits the file wins.
fn locate(bytes: &[u8]) -> Result<Directory, String> {
    if bytes.len() < 22 {
        return Err("too short for an end record".into());
    }
    let low = bytes.len().saturating_sub(22 + 65_535);
    let mut pos = bytes.len() - 22;
    loop {
        if bytes[pos..].starts_with(EOCD)
            && let Some(dir) = from_eocd(bytes, pos)
        {
            return Ok(dir);
        }
        if pos == low {
            return Err("no end record with a directory that fits the file".into());
        }
        pos -= 1;
    }
}

fn from_eocd(bytes: &[u8], pos: usize) -> Option<Directory> {
    let count = u64::from(u16_at(bytes, pos + 10)?);
    let size = u64::from(u32_at(bytes, pos + 12)?);
    let offset = u64::from(u32_at(bytes, pos + 16)?);
    let saturated = count == 0xFFFF || size == 0xFFFF_FFFF || offset == 0xFFFF_FFFF;
    let locator = pos
        .checked_sub(20)
        .filter(|&l| bytes[l..].starts_with(LOCATOR));
    // A valid ZIP64 record wins even when the classic fields are not saturated, as in
    // readers; some writers always emit one.
    if let Some(loc) = locator {
        if let Some(dir) = from_locator(bytes, loc) {
            return Some(dir);
        }
        if saturated {
            return None;
        }
    }
    if size == 0xFFFF_FFFF || offset == 0xFFFF_FFFF {
        return None;
    }
    // A saturated 16-bit count without ZIP64 records: walk until the directory ends.
    let count = (count != 0xFFFF).then_some(count);
    directory(bytes, pos as u64, count, size, offset, None)
}

/// ZIP64: the end record is searched from the locator's offset up to the locator, as readers
/// do when a stub or junk shifts the archive.
///
/// The search covers only 64 KiB after the claimed offset, so the work stays bounded. When the
/// record is moved further (bytes inside the archive, or a stub of more than 64 KiB in front of
/// a self-extracting archive whose offsets were not adjusted), this returns None. That is
/// safe: with unsaturated classic fields the caller falls back to them and the gap shift still
/// finds the directory; only a saturated classic record then gives "not readable" (Zip,
/// Likely, or Unknown for a file behind a stub), although full readers may open it.
fn from_locator(bytes: &[u8], loc: usize) -> Option<Directory> {
    let claimed = usize::try_from(u64_at(bytes, loc + 8)?).ok()?;
    let from = claimed.min(loc);
    let to = loc.min(from.saturating_add(EOCD64_SEARCH));
    (from..to)
        .filter(|&p| bytes[p..].starts_with(EOCD64))
        .find_map(|p| {
            let count = u64_at(bytes, p + 32)?;
            let size = u64_at(bytes, p + 40)?;
            let offset = u64_at(bytes, p + 48)?;
            // Bytes inserted in front of the record move it away from where the locator
            // says it is; the same shift then applies to the directory.
            let moved = (p as u64).checked_sub(claimed as u64);
            directory(bytes, p as u64, Some(count), size, offset, moved)
        })
}

/// Finds where a directory of `size` bytes at `offset` really starts, given the record after
/// it at `anchor`. Candidate shifts, in order: the one derived from a moved ZIP64 record,
/// none, and the gap between the directory end and `anchor` (bytes in front of the archive).
/// The first candidate that lands on a central directory header (or an empty directory)
/// wins. No shift comes before the gap: bytes between the directory and `anchor` can make
/// the gap land on a later header, while a stub's bytes at the raw offset are not a header.
fn directory(
    bytes: &[u8],
    anchor: u64,
    count: Option<u64>,
    size: u64,
    offset: u64,
    moved: Option<u64>,
) -> Option<Directory> {
    let end = offset.checked_add(size)?;
    let gap = anchor.checked_sub(end);
    [moved, Some(0), gap]
        .into_iter()
        .flatten()
        .find_map(|shift| {
            let start = offset.checked_add(shift)?;
            if start.checked_add(size)? > anchor {
                return None;
            }
            let at = usize::try_from(start).ok()?;
            let fits = if count == Some(0) || size == 0 {
                size == 0 && count.is_none_or(|n| n == 0)
            } else {
                bytes.get(at..)?.starts_with(CENTRAL)
            };
            fits.then_some(Directory {
                start,
                size,
                count,
                shift,
            })
        })
}

/// Parses one central directory header at `p`: the entry, its raw name and the next offset.
fn central_header(bytes: &[u8], p: u64, shift: u64) -> Option<(Entry, &[u8], u64)> {
    let at = usize::try_from(p).ok()?;
    if !bytes.get(at..)?.starts_with(CENTRAL) {
        return None;
    }
    let flags = u16_at(bytes, at + 8)?;
    let method = u16_at(bytes, at + 10)?;
    let mut compressed = u64::from(u32_at(bytes, at + 20)?);
    let mut uncompressed = u64::from(u32_at(bytes, at + 24)?);
    let name_len = usize::from(u16_at(bytes, at + 28)?);
    let extra_len = usize::from(u16_at(bytes, at + 30)?);
    let comment_len = usize::from(u16_at(bytes, at + 32)?);
    let mut local = u64::from(u32_at(bytes, at + 42)?);
    let name = bytes.get(at + 46..at + 46 + name_len)?;
    let extra = bytes.get(at + 46 + name_len..at + 46 + name_len + extra_len)?;
    if uncompressed == 0xFFFF_FFFF || compressed == 0xFFFF_FFFF || local == 0xFFFF_FFFF {
        // ZIP64 extended information (id 1): the saturated fields in this order.
        let mut e = 0;
        while e + 4 <= extra.len() {
            let id = u16_at(extra, e)?;
            let len = usize::from(u16_at(extra, e + 2)?);
            let data = extra.get(e + 4..e + 4 + len)?;
            if id == 1 {
                let mut k = 0;
                for field in [&mut uncompressed, &mut compressed, &mut local] {
                    if *field == 0xFFFF_FFFF {
                        *field = u64_at(data, k)?;
                        k += 8;
                    }
                }
                break;
            }
            e += 4 + len;
        }
    }
    let next = p + (46 + name_len + extra_len + comment_len) as u64;
    let entry = Entry {
        flags,
        method,
        compressed,
        uncompressed,
        local: local.checked_add(shift)?,
    };
    Some((entry, name, next))
}

/// Reads an entry's data, stored or deflated, within `budget` (which shrinks by what was
/// read). None when the entry is encrypted, uses another method, is larger than the budget
/// or is broken. Nothing larger than the budget is copied: a stored entry must declare the
/// same compressed and uncompressed size, and deflate input stops after
/// [`MAX_INFLATE_INPUT`] bytes.
pub(crate) fn read(bytes: &[u8], entry: &Entry, budget: &mut u64) -> Option<Vec<u8>> {
    if entry.flags & 1 != 0 || entry.uncompressed > *budget {
        return None;
    }
    if entry.method == 0 && entry.compressed != entry.uncompressed {
        return None;
    }
    let at = usize::try_from(entry.local).ok()?;
    if !bytes.get(at..)?.starts_with(LOCAL) {
        return None;
    }
    let name_len = usize::from(u16_at(bytes, at + 26)?);
    let extra_len = usize::from(u16_at(bytes, at + 28)?);
    let data = at + 30 + name_len + extra_len;
    let len = usize::try_from(entry.compressed).ok()?;
    let raw = bytes.get(data..data.checked_add(len)?)?;
    let out = match entry.method {
        0 => raw.to_vec(),
        8 => {
            let mut out = Vec::new();
            // One byte more than allowed tells a full read from a cut one.
            let input = &raw[..raw.len().min(MAX_INFLATE_INPUT)];
            flate2::read::DeflateDecoder::new(input)
                .take(*budget + 1)
                .read_to_end(&mut out)
                .ok()?;
            out
        }
        _ => return None,
    };
    let read = out.len() as u64;
    if read > *budget || read != entry.uncompressed {
        return None;
    }
    *budget -= read;
    Some(out)
}

pub(crate) fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

pub(crate) fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

pub(crate) fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    use super::*;

    /// Records the largest single allocation made by the current thread while armed.
    struct Largest;

    thread_local! {
        static ARMED: Cell<bool> = const { Cell::new(false) };
        static LARGEST: Cell<usize> = const { Cell::new(0) };
    }

    // SAFETY: forwards to the system allocator; only reads and writes thread-local cells.
    unsafe impl GlobalAlloc for Largest {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let _ = ARMED.try_with(|armed| {
                if armed.get() {
                    let _ = LARGEST.try_with(|l| l.set(l.get().max(layout.size())));
                }
            });
            // SAFETY: same layout contract as the caller's.
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: `ptr` came from `System.alloc` with this layout.
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    #[global_allocator]
    static GLOBAL: Largest = Largest;

    /// Runs `f` and returns its result with the largest allocation it made.
    fn largest_allocation<T>(f: impl FnOnce() -> T) -> (T, usize) {
        LARGEST.with(|l| l.set(0));
        ARMED.with(|a| a.set(true));
        let out = f();
        ARMED.with(|a| a.set(false));
        (out, LARGEST.with(Cell::get))
    }

    /// A local header for `name` followed by `data`, at offset 0.
    fn local(name: &str, data: &[u8]) -> Vec<u8> {
        let mut out = LOCAL.to_vec();
        out.extend_from_slice(&[0u8; 22]);
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        out
    }

    #[test]
    fn stored_entry_is_not_copied_beyond_its_size() {
        let mut data = b"application/vnd.oasis.opendocument.text".to_vec();
        data.resize(1 << 20, b'x');
        let bytes = local("mimetype", &data);
        let entry = Entry {
            flags: 0,
            method: 0,
            compressed: data.len() as u64,
            uncompressed: 39,
            local: 0,
        };
        let mut budget = 64 * 1024;
        let (out, largest) = largest_allocation(|| read(&bytes, &entry, &mut budget));
        assert!(out.is_none());
        assert!(largest < 64 * 1024, "allocated {largest} bytes");
        assert_eq!(budget, 64 * 1024);
    }

    #[test]
    fn stored_entry_within_budget_is_read() {
        let bytes = local("mimetype", b"application/epub+zip");
        let entry = Entry {
            flags: 0,
            method: 0,
            compressed: 20,
            uncompressed: 20,
            local: 0,
        };
        let mut budget = 64 * 1024;
        assert_eq!(
            read(&bytes, &entry, &mut budget).as_deref(),
            Some(&b"application/epub+zip"[..])
        );
        assert_eq!(budget, 64 * 1024 - 20);
    }

    /// `empty` empty stored deflate blocks, then a final stored block with `data`.
    fn deflate_with_empty_blocks(empty: usize, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(empty * 5 + data.len() + 5);
        for _ in 0..empty {
            out.extend_from_slice(&[0x00, 0x00, 0x00, 0xFF, 0xFF]);
        }
        let len = data.len() as u16;
        out.push(0x01);
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(data);
        out
    }

    fn deflated(stream: &[u8], uncompressed: u64) -> (Vec<u8>, Entry) {
        let bytes = local("mimetype", stream);
        let entry = Entry {
            flags: 0,
            method: 8,
            compressed: stream.len() as u64,
            uncompressed,
            local: 0,
        };
        (bytes, entry)
    }

    #[test]
    fn deflate_reads_at_most_one_mib_of_input() {
        // 2 MiB of empty blocks before the data: input past 1 MiB is never read.
        let stream = deflate_with_empty_blocks(2 * 1024 * 1024 / 5, b"hello");
        let (bytes, entry) = deflated(&stream, 5);
        let mut budget = 64 * 1024;
        assert!(read(&bytes, &entry, &mut budget).is_none());
        // A few empty blocks are fine.
        let stream = deflate_with_empty_blocks(10, b"hello");
        let (bytes, entry) = deflated(&stream, 5);
        assert_eq!(
            read(&bytes, &entry, &mut budget).as_deref(),
            Some(&b"hello"[..])
        );
    }
}
