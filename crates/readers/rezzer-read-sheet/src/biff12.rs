// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A bounded walk over the BIFF12 records of XLSB parts, run before calamine reads them.
//!
//! The record layout is from the MS-XLSB specification. The walk follows the records the way
//! calamine 0.37.0 reads them, so it sees what calamine will see:
//! - from `workbook.bin`: the sheets (state, relationship id, name) and the defined names
//!   with their formula tokens;
//! - from a sheet part: where its formula records are, the cached error result of each
//!   formula whose result is an error (calamine's cell reader skips those), whether every
//!   formula token stream is safe to hand to calamine's parser, and the merged cells, which
//!   calamine does not read for XLSB.
//!
//! calamine's formula parser recurses once per nested `PtgMemFunc` token and copies the text
//! built so far on most operators, so a crafted token stream can overflow the stack or take
//! quadratic time. The walk bounds the nesting, the text a token stream can produce and the
//! bytes the parser moves, without building any text.

use std::collections::{HashMap, HashSet};

/// Nested `PtgMemFunc` levels allowed. Excel nests functions 64 deep.
const MAX_NESTING: usize = 64;

/// Most bytes of formula text one token stream may produce.
pub(crate) const MAX_FORMULA_OUT: u64 = 1 << 20;

/// Most bytes calamine's parser may move (copies and inserts) for one token stream. The
/// walk counts one copy of the text so far per operator; a function token copies its
/// arguments twice, so this bound is approximate (within a factor of two), a time bound
/// rather than an exact one.
const MAX_WORK: u64 = 1 << 28;

/// Most bytes calamine's parser may move for all defined names together: they are parsed
/// when the workbook is opened, where no time limit can stop it.
pub(crate) const MAX_OPEN_WORK: u64 = 1 << 30;

/// Function name and argument separators of one function token, at most.
const FUNCTION_OUT: u64 = 32 + 2 + 256;

/// The function table size of calamine 0.37.0 (`FTAB_LEN`).
const FTAB_LEN: usize = 485;

/// Records of a part, read the way calamine reads them.
struct Records<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Records<'a> {
    fn new(buf: &'a [u8]) -> Records<'a> {
        Records { buf, pos: 0 }
    }

    fn byte(&mut self) -> Option<u8> {
        let b = *self.buf.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    /// The next record type; None at the end of the part.
    fn kind(&mut self) -> Option<u16> {
        let b = self.byte()?;
        if b & 0x80 == 0x80 {
            let b2 = self.byte()?;
            Some(u16::from(b & 0x7F) + (u16::from(b2 & 0x7F) << 7))
        } else {
            Some(u16::from(b))
        }
    }

    /// The body of the record whose type was just read.
    fn body(&mut self) -> Result<&'a [u8], String> {
        let at = self.pos;
        let mut b = self.byte().ok_or("a record ends inside its length")?;
        let mut len = usize::from(b & 0x7F);
        for i in 1..4 {
            if b & 0x80 == 0 {
                break;
            }
            b = self.byte().ok_or("a record ends inside its length")?;
            len += usize::from(b & 0x7F) << (7 * i);
        }
        let left = self.buf.len() - self.pos;
        if len > left {
            return Err(format!(
                "the record at byte {at} claims {len} bytes, only {left} are left"
            ));
        }
        let data = &self.buf[self.pos..self.pos + len];
        self.pos += len;
        Ok(data)
    }

    /// The next record, type and body; None at the end.
    fn next(&mut self) -> Option<Result<(u16, &'a [u8]), String>> {
        let kind = self.kind()?;
        Some(self.body().map(|b| (kind, b)))
    }
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// An `XLWideString` at `at`: the text and the bytes it takes.
fn wide(b: &[u8], at: usize) -> Option<(String, usize)> {
    let len = u32_at(b, at)? as usize;
    let bytes = b.get(at + 4..at.checked_add(4)?.checked_add(len.checked_mul(2)?)?)?;
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    Some((String::from_utf16_lossy(&units), 4 + len * 2))
}

/// A sheet as `workbook.bin` lists it.
#[derive(Debug, Clone)]
pub(crate) struct BookSheet {
    pub name: String,
    /// The relationship id, None when the record has none (calamine then skips the sheet).
    pub rel_id: Option<String>,
}

/// What `workbook.bin` holds for the checks.
#[derive(Debug, Default)]
pub(crate) struct Book {
    pub sheets: Vec<BookSheet>,
    /// Defined names with their formula tokens, as calamine parses them at open.
    pub names: Vec<(String, Vec<u8>)>,
    /// The defined names whose `fHidden` bit is set, lower case.
    pub hidden_names: HashSet<String>,
    /// The defined names whose `fBuiltin` bit is set, lower case.
    pub builtin_names: HashSet<String>,
    /// The most 3D reference entries (`BrtExternSheet`) one record lists; calamine copies a
    /// sheet name for each when it opens the workbook.
    pub extern_entries: u64,
}

/// Records calamine takes as the end of the defined names.
const AFTER_NAMES: [u16; 9] = [
    0x009D, 0x0225, 0x018D, 0x0180, 0x009A, 0x0252, 0x0229, 0x009B, 0x0084,
];

/// Reads the sheets and defined names of `workbook.bin`. An error means calamine cannot read
/// the part either, or would read it past its end.
pub(crate) fn read_book(bytes: &[u8]) -> Result<Book, String> {
    let mut book = Book::default();
    let mut r = Records::new(bytes);
    loop {
        let Some(rec) = r.next() else {
            return Err("the part ends before the list of sheets does".into());
        };
        let (kind, body) = rec?;
        match kind {
            0x009C => {
                // BrtBundleSh: hsState, iTabID, strRelID, strName (the state comes from calamine).
                let bad = || "a sheet record is too short".to_string();
                let rel_len = u32_at(body, 8).ok_or_else(bad)?;
                let (rel_id, name_at) = if rel_len == 0xFFFF_FFFF {
                    (None, 12)
                } else {
                    let (id, n) = wide(body, 8).ok_or_else(bad)?;
                    (Some(id), 8 + n)
                };
                let (name, _) = wide(body, name_at).ok_or_else(bad)?;
                book.sheets.push(BookSheet { name, rel_id });
            }
            0x0090 => break,
            _ => {}
        }
    }
    // calamine reads only the type of records it does not know in this part, then reads
    // on from the next byte; the walk does the same so it meets the same name records.
    loop {
        let Some(kind) = r.kind() else {
            return Err("the part ends before the defined names do".into());
        };
        match kind {
            0x016A => {
                let body = r.body()?;
                let count = u32_at(body, 0).ok_or("a 3D reference table record is too short")?;
                // calamine reads the entries from its buffer, which may hold bytes of an
                // earlier record past this one.
                if 4 + 12 * u64::from(count) > body.len() as u64 {
                    return Err(format!(
                        "a 3D reference table lists {count} entries, more than its record holds"
                    ));
                }
                book.extern_entries = book.extern_entries.max(u64::from(count));
            }
            0x0027 => {
                let body = r.body()?;
                let bad = || "a defined name record is too short".to_string();
                let (name, n) = wide(body, 9).ok_or_else(bad)?;
                let len = u32_at(body, 9 + n).ok_or_else(bad)? as usize;
                let rgce = body
                    .get(13 + n..(13 + n).saturating_add(len))
                    .ok_or_else(bad)?;
                // BrtName (MS-XLSB 2.4.718): bit A of its flags is fHidden, bit F fBuiltin.
                let flags = u32_at(body, 0).unwrap_or(0);
                if flags & 1 != 0 {
                    book.hidden_names.insert(name.to_lowercase());
                }
                if flags & (1 << 5) != 0 {
                    book.builtin_names.insert(name.to_lowercase());
                }
                book.names.push((name, rgce.to_vec()));
            }
            k if AFTER_NAMES.contains(&k) => return Ok(book),
            _ => {}
        }
    }
}

/// The longest texts a token stream can name: a sheet in a 3D reference, a defined name.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Widths {
    pub sheet: u64,
    pub name: u64,
}

/// Why a token stream is not handed to calamine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unsafe {
    Nesting,
    Text,
    Work,
    /// The tokens run past the end of their record: calamine would read bytes left over in
    /// its buffer from an earlier record.
    Truncated,
}

impl Unsafe {
    pub fn what(self) -> String {
        match self {
            Unsafe::Nesting => format!("formula tokens nested deeper than {MAX_NESTING} levels"),
            Unsafe::Text => format!("a formula would be longer than {MAX_FORMULA_OUT} bytes"),
            Unsafe::Work => "a formula would take too long to rebuild as text".to_string(),
            Unsafe::Truncated => "a formula record is shorter than its tokens".to_string(),
        }
    }
}

struct Walk {
    widths: Widths,
    out: u64,
    work: u64,
    max_work: u64,
}

impl Walk {
    fn add(&mut self, n: u64) -> Result<(), Unsafe> {
        self.out = self.out.saturating_add(n);
        if self.out > MAX_FORMULA_OUT {
            return Err(Unsafe::Text);
        }
        Ok(())
    }

    /// One copy or insert over the text built so far.
    fn shift(&mut self) -> Result<(), Unsafe> {
        self.work = self.work.saturating_add(self.out);
        if self.work > self.max_work {
            return Err(Unsafe::Work);
        }
        Ok(())
    }

    /// Walks the tokens like calamine's `parse_formula`. A token calamine cannot read ends
    /// the walk: calamine stops there too, with an error.
    fn tokens(&mut self, mut rgce: &[u8], depth: usize) -> Result<(), Unsafe> {
        if depth > MAX_NESTING {
            return Err(Unsafe::Nesting);
        }
        let sheet = self.widths.sheet;
        while let Some((&ptg, rest)) = rgce.split_first() {
            rgce = rest;
            let (skip, out, shift) = match ptg {
                0x3a | 0x5a | 0x7a => (8, sheet + 17, false),
                0x3b | 0x5b | 0x7b => (14, sheet + 34, false),
                0x3c | 0x5c | 0x7c => (8, sheet + 6, false),
                0x3d | 0x5d | 0x7d => (14, sheet + 6, false),
                0x01 => (4, 0, false),
                0x03..=0x11 => (0, 2, true),
                0x12 | 0x13 => (0, 1, true),
                0x14 => (0, 1, false),
                0x15 => (0, 2, true),
                0x16 => (0, 0, false),
                0x17 => {
                    let Some(cch) = u16_at(rgce, 0) else {
                        return Ok(());
                    };
                    (2 + 2 * usize::from(cch), 2 + 3 * u64::from(cch), false)
                }
                0x18 => match rgce.first() {
                    Some(0x19) => (13, 0, false),
                    Some(0x1D) => (5, 0, false),
                    _ => return Ok(()),
                },
                0x19 => match rgce.first() {
                    Some(0x01 | 0x02 | 0x08 | 0x20 | 0x21 | 0x40 | 0x41 | 0x80) => (3, 0, false),
                    Some(0x04) => (11, 0, false),
                    Some(0x10) => (3, 5, true),
                    _ => return Ok(()),
                },
                0x1C => (1, 16, false),
                0x1D => (1, 5, false),
                0x1E => (2, 5, false),
                // A float is written in full, without an exponent: up to about 330 bytes.
                0x1F => (8, 340, false),
                0x20 | 0x40 | 0x60 => (14, 0, false),
                0x21 | 0x41 | 0x61 => {
                    match u16_at(rgce, 0) {
                        Some(i) if usize::from(i) < FTAB_LEN => {}
                        _ => return Ok(()),
                    }
                    (2, FUNCTION_OUT, true)
                }
                0x22 | 0x42 | 0x62 => {
                    match u16_at(rgce, 1) {
                        Some(i) if usize::from(i) < FTAB_LEN => {}
                        _ => return Ok(()),
                    }
                    (3, FUNCTION_OUT, true)
                }
                0x23 | 0x43 | 0x63 => (4, self.widths.name, false),
                0x24 | 0x44 | 0x64 => (6, 16, false),
                0x25 | 0x45 | 0x65 => (12, 33, false),
                0x2A | 0x4A | 0x6A => (6, 5, false),
                0x2B | 0x4B | 0x6B => (12, 5, false),
                0x29 | 0x49 | 0x69 => {
                    let Some(cce) = u16_at(rgce, 0) else {
                        return Ok(());
                    };
                    let cce = usize::from(cce);
                    let Some(inner) = rgce.get(2..2 + cce) else {
                        return Ok(());
                    };
                    let before = self.out;
                    self.tokens(inner, depth + 1)?;
                    // The inner text is built apart, then copied in.
                    self.add(self.out - before)?;
                    self.shift()?;
                    rgce = &rgce[2 + cce..];
                    continue;
                }
                0x39 | 0x59 | 0x79 => (6, 16, false),
                _ => return Ok(()),
            };
            self.add(out)?;
            if shift {
                self.shift()?;
            }
            let Some(rest) = rgce.get(skip..) else {
                return Ok(());
            };
            rgce = rest;
        }
        Ok(())
    }
}

/// Checks that calamine can parse a token stream within the bounds. Returns the most bytes
/// of text it can produce.
pub(crate) fn check_tokens(rgce: &[u8], widths: Widths) -> Result<u64, Unsafe> {
    check_tokens_within(rgce, widths, MAX_WORK).map(|(out, _)| out)
}

/// Like [`check_tokens`], with at most `max_work` bytes moved (and never more than for one
/// formula). Returns the text bound and the bytes moved.
pub(crate) fn check_tokens_within(
    rgce: &[u8],
    widths: Widths,
    max_work: u64,
) -> Result<(u64, u64), Unsafe> {
    let mut walk = Walk {
        widths,
        out: 0,
        work: 0,
        max_work: max_work.min(MAX_WORK),
    };
    walk.tokens(rgce, 0)?;
    Ok((walk.out, walk.work))
}

/// What a sheet part holds for the checks, in calamine's reading order.
#[derive(Debug, Default)]
pub(crate) struct SheetScan {
    /// The cell of every formula record calamine's formula reader returns, in order.
    pub formulas: Vec<(u32, u32)>,
    /// The cached error of formula cells whose result is an error.
    pub error_results: HashMap<(u32, u32), &'static str>,
    /// Why calamine must not parse the formulas of this sheet, at which cell.
    pub unsafe_formula: Option<(Unsafe, (u32, u32))>,
    /// The merged cells (`BrtMergeCell`, after the sheet data) as 0-based
    /// (rwFirst, rwLast, colFirst, colLast), in the order of the part.
    pub merges: Vec<(u32, u32, u32, u32)>,
    /// Merge records left out: shorter than their 16 bytes, or past the last row or column
    /// of a sheet.
    pub merges_bad: u64,
    /// Merge records past the cap, not listed.
    pub merges_cut: u64,
    /// The records after the sheet data could not be walked to the end: the merges from
    /// there on are not read. The cells are not affected.
    pub merge_error: Option<String>,
}

/// The last 0-based row and column of a sheet.
const LAST_ROW: u32 = 1_048_575;
const LAST_COL: u32 = 16_383;

/// The error text of a `BErr` code, as calamine writes it.
fn error_text(code: u8) -> Option<&'static str> {
    Some(match code {
        0x00 => "#NULL!",
        0x07 => "#DIV/0!",
        0x0F => "#VALUE!",
        0x17 => "#REF!",
        0x1D => "#NAME?",
        0x24 => "#NUM!",
        0x2A => "#N/A",
        0x2B => "#DATA!",
        _ => return None,
    })
}

/// The bytes calamine's cell reader reads from a record of this type: the column and style
/// of every cell, then the value; a string cell its length and characters.
fn cell_fields(kind: u16, body: &[u8]) -> Option<usize> {
    Some(match kind {
        0x0000 => 4,
        0x0002 | 0x0007 => 12,
        0x0003 | 0x0004 | 0x000A => 9,
        0x0005 | 0x0009 => 16,
        0x000B => 11,
        0x0006 | 0x0008 => match u32_at(body, 8) {
            Some(cch) => 12 + 2 * cch as usize,
            None => 12,
        },
        _ => return None,
    })
}

/// The token stream of a formula cell record: (type, body) to rgce, as calamine finds it.
fn record_tokens(kind: u16, body: &[u8]) -> Option<&[u8]> {
    let at = match kind {
        0x0008 => 14 + 2 * u32_at(body, 8)? as usize,
        0x0009 => 18,
        0x000A | 0x000B => 11,
        _ => return None,
    };
    let len = u32_at(body, at)? as usize;
    body.get(at + 4..(at + 4).checked_add(len)?)
}

/// Walks a sheet part the way calamine's cell reader does. An error means a record runs
/// past the end of the part or the part ends before its cells. After the cells the walk goes
/// on to `BrtEndSheet` for the merged cells, at most `max_merges` of them; calamine reads
/// none of those records.
pub(crate) fn scan_sheet(
    bytes: &[u8],
    widths: Widths,
    max_merges: u64,
) -> Result<SheetScan, String> {
    let mut out = SheetScan::default();
    let mut r = Records::new(bytes);
    let early = || "the part ends before its cells".to_string();
    // BrtWsDim, then BrtBeginSheetData with the blocks calamine skips on the way.
    loop {
        let (kind, _) = r.next().ok_or_else(early)??;
        if kind == 0x0094 {
            break;
        }
    }
    let ends: [(u16, u16); 3] = [(0x0085, 0x0086), (0x0025, 0x0026), (0x0186, 0x0187)];
    loop {
        let (kind, _) = r.next().ok_or_else(early)??;
        if kind == 0x0091 {
            break;
        }
        if let Some(&(_, end)) = ends.iter().find(|(start, _)| *start == kind) {
            loop {
                let k = r.kind().ok_or_else(early)?;
                r.body()?;
                if k == end {
                    break;
                }
            }
        }
    }
    let mut row = 0u32;
    let mut past_rows = false;
    while let Some(rec) = r.next() {
        let (kind, body) = rec?;
        // calamine reads the fields of a cell record from a buffer that keeps the bytes of
        // earlier records: a record shorter than its fields would give leftover values.
        if let Some(need) = cell_fields(kind, body)
            && body.len() < need
        {
            return Err(format!(
                "a cell record of type {kind:#06x} in row {} is shorter than its fields ({} of {need} bytes)",
                u64::from(row) + 1,
                body.len()
            ));
        }
        match kind {
            0x0000 => {
                row = u32_at(body, 0).unwrap_or(0);
                if row > 0x0010_0000 {
                    // calamine stops reading cells here; the merges come after the cells.
                    past_rows = true;
                    break;
                }
            }
            0x0092 => break,
            0x0008..=0x000B => {
                let col = u32_at(body, 0).unwrap_or(0);
                out.formulas.push((row, col));
                if kind == 0x000B
                    && let Some(e) = body.get(8).copied().and_then(error_text)
                {
                    out.error_results.insert((row, col), e);
                }
                if out.unsafe_formula.is_some() {
                    continue;
                }
                match record_tokens(kind, body).map(|rgce| check_tokens(rgce, widths)) {
                    Some(Ok(_)) => {}
                    Some(Err(why)) => out.unsafe_formula = Some((why, (row, col))),
                    None => out.unsafe_formula = Some((Unsafe::Truncated, (row, col))),
                }
            }
            _ => {}
        }
    }
    // Records calamine does not read never make the sheet unreadable: a problem there only
    // ends the merges.
    if past_rows && let Err(e) = skip_cells(&mut r) {
        out.merge_error = Some(e);
        return Ok(out);
    }
    merges(&mut r, &mut out, max_merges);
    Ok(out)
}

/// Walks on to the end of the sheet data (`BrtEndSheetData`).
fn skip_cells(r: &mut Records) -> Result<(), String> {
    while let Some(rec) = r.next() {
        let (kind, _) = rec?;
        if kind == 0x0092 {
            break;
        }
    }
    Ok(())
}

/// The `BrtMergeCell` records between the sheet data and `BrtEndSheet` (MS-XLSB 2.4.713:
/// rwFirst, rwLast, colFirst, colLast, 4 bytes each). A record that runs past the end of the
/// part ends the walk with `merge_error`.
fn merges(r: &mut Records, out: &mut SheetScan, max_merges: u64) {
    while let Some(rec) = r.next() {
        let (kind, body) = match rec {
            Ok(k) => k,
            Err(e) => {
                out.merge_error = Some(e);
                return;
            }
        };
        match kind {
            0x00B0 => {
                let field = |at| u32_at(body, at);
                let (Some(r0), Some(r1), Some(c0), Some(c1)) =
                    (field(0), field(4), field(8), field(12))
                else {
                    out.merges_bad += 1;
                    continue;
                };
                if r0.max(r1) > LAST_ROW || c0.max(c1) > LAST_COL {
                    out.merges_bad += 1;
                } else if out.merges.len() as u64 >= max_merges {
                    out.merges_cut += 1;
                } else {
                    out.merges.push((r0, r1, c0, c1));
                }
            }
            0x0082 => return,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: Widths = Widths {
        sheet: 31,
        name: 255,
    };

    fn nested(depth: usize) -> Vec<u8> {
        let mut inner = vec![0x1E, 1, 0];
        for _ in 0..depth {
            let mut p = vec![0x29];
            p.extend((inner.len() as u16).to_le_bytes());
            p.extend(inner);
            inner = p;
        }
        inner
    }

    #[test]
    fn nesting_is_bounded() {
        assert!(check_tokens(&nested(10), W).is_ok());
        assert_eq!(check_tokens(&nested(200), W), Err(Unsafe::Nesting));
    }

    #[test]
    fn quadratic_rebuilds_are_bounded() {
        // A long string, then a hundred thousand parentheses around it.
        let mut rgce = vec![0x17];
        rgce.extend(255u16.to_le_bytes());
        rgce.extend([b'a', 0].repeat(255));
        rgce.extend([0x15].repeat(100_000));
        assert!(matches!(
            check_tokens(&rgce, W),
            Err(Unsafe::Work | Unsafe::Text)
        ));
        // Names are as long as the longest name.
        let mut names = Vec::new();
        for _ in 0..10 {
            names.extend([0x23, 1, 0, 0, 0]);
        }
        assert_eq!(
            check_tokens(
                &names,
                Widths {
                    sheet: 31,
                    name: 100
                }
            ),
            Ok(1_000)
        );
    }

    #[test]
    fn the_work_of_all_names_shares_one_budget() {
        // `1+1+1...`: each operator copies the text built so far.
        let mut rgce = vec![0x1E, 1, 0];
        for _ in 0..200 {
            rgce.extend([0x1E, 1, 0, 0x03]);
        }
        let (_, work) = check_tokens_within(&rgce, W, u64::MAX).unwrap();
        assert!(work > 1_000);
        assert_eq!(check_tokens_within(&rgce, W, work - 1), Err(Unsafe::Work));
    }

    #[test]
    fn text_bounds_cover_the_longest_references() {
        // A 3D reference: sheet, `!`, `$`, four column letters, `$`, ten row digits.
        let mut r3d = vec![0x3a, 0, 0];
        r3d.extend(u32::MAX.to_le_bytes());
        r3d.extend(u16::MAX.to_le_bytes());
        assert_eq!(check_tokens(&r3d, W), Ok(31 + 17));
        let mut a3d = vec![0x3b, 0, 0];
        a3d.extend([0xFF; 12]);
        assert_eq!(check_tokens(&a3d, W), Ok(31 + 34));
        let mut area = vec![0x25];
        area.extend([0xFF; 12]);
        assert_eq!(check_tokens(&area, W), Ok(33));
    }

    #[test]
    fn short_value_records_are_errors() {
        let sheet = |cell: Vec<u8>| {
            let mut b = vec![0x94, 0x01, 16];
            b.extend([0; 16]);
            b.extend([0x91, 0x01, 0]);
            b.extend([0x00, 4, 0, 0, 0, 0]);
            b.extend(cell);
            b
        };
        // A real number needs 16 bytes, a shared string index 12.
        assert!(scan_sheet(&sheet(vec![0x05, 8, 0, 0, 0, 0, 0, 0, 0, 0]), W, 100).is_err());
        assert!(scan_sheet(&sheet(vec![0x07, 8, 0, 0, 0, 0, 0, 0, 0, 0]), W, 100).is_err());
        let mut ok = vec![0x05, 16];
        ok.extend([0; 16]);
        assert!(scan_sheet(&sheet(ok), W, 100).is_ok());
    }

    /// A sheet part with one row record, then `tail` after BrtEndSheetData.
    fn sheet_with_tail(tail: &[u8]) -> Vec<u8> {
        let mut b = vec![0x94, 0x01, 16];
        b.extend([0; 16]);
        b.extend([0x91, 0x01, 0]);
        b.extend([0x00, 4, 0, 0, 0, 0]);
        b.extend([0x92, 0x01, 0]);
        b.extend(tail);
        b
    }

    /// BrtMergeCell: rwFirst, rwLast, colFirst, colLast.
    fn merge(r0: u32, r1: u32, c0: u32, c1: u32) -> Vec<u8> {
        let mut b = vec![0xB0, 0x01, 16];
        for v in [r0, r1, c0, c1] {
            b.extend(v.to_le_bytes());
        }
        b
    }

    #[test]
    fn merges_after_the_sheet_data_are_read() {
        let mut tail = vec![0xB1, 0x01, 4, 2, 0, 0, 0];
        tail.extend(merge(0, 0, 1, 2));
        tail.extend(merge(3, 5, 0, 0));
        // Past the sheet's last row or column: not a merge.
        tail.extend(merge(0, 1_048_576, 0, 0));
        // Too short: skipped and counted.
        tail.extend([0xB0, 0x01, 4, 0, 0, 0, 0]);
        tail.extend([0xB2, 0x01, 0, 0x82, 0x01, 0]);
        let scan = scan_sheet(&sheet_with_tail(&tail), W, 100).unwrap();
        assert_eq!(scan.merges, [(0, 0, 1, 2), (3, 5, 0, 0)]);
        assert_eq!(scan.merges_bad, 2);
        assert!(scan.merge_error.is_none());
        assert_eq!(scan.merges_cut, 0);
    }

    #[test]
    fn merges_are_capped_and_a_broken_tail_keeps_the_sheet() {
        let mut tail = Vec::new();
        for r in 0..5 {
            tail.extend(merge(r * 2, r * 2 + 1, 0, 0));
        }
        let scan = scan_sheet(&sheet_with_tail(&tail), W, 3).unwrap();
        assert_eq!(scan.merges.len(), 3);
        assert_eq!(scan.merges_cut, 2);

        let mut tail = merge(0, 1, 0, 0);
        tail.extend([0xB0, 0x01, 0x7F, 1, 2]);
        let scan = scan_sheet(&sheet_with_tail(&tail), W, 100).unwrap();
        assert_eq!(scan.merges, [(0, 1, 0, 0)]);
        assert!(scan.merge_error.is_some());
    }

    #[test]
    fn hidden_names_are_marked() {
        let rec = |kind: u8, body: &[u8]| {
            let mut b = vec![kind | 0x80, 0x01, body.len() as u8];
            b.extend(body);
            b
        };
        let wide = |s: &str| {
            let mut b = (s.len() as u32).to_le_bytes().to_vec();
            for u in s.encode_utf16() {
                b.extend(u.to_le_bytes());
            }
            b
        };
        let mut sheet = vec![0; 8];
        sheet.extend(wide("rId1"));
        sheet.extend(wide("S"));
        let mut bytes = rec(0x1C, &sheet); // BrtBundleSh, 0x009C
        bytes.extend(rec(0x10, &[])); // BrtEndBundleShs, 0x0090
        for (name, flags) in [("Shown", 0u32), ("Secret", 1), ("Print_Area", 1 << 5)] {
            let mut body = flags.to_le_bytes().to_vec();
            body.extend([0; 5]);
            body.extend(wide(name));
            body.extend(0u32.to_le_bytes());
            bytes.extend([0x27, body.len() as u8]);
            bytes.extend(body);
        }
        bytes.extend(rec(0x1D, &[0; 4])); // BrtCalcProp, 0x009D
        let book = read_book(&bytes).unwrap();
        assert_eq!(book.names.len(), 3);
        assert!(book.hidden_names.contains("secret"));
        assert!(!book.hidden_names.contains("shown"));
        assert!(!book.hidden_names.contains("print_area"));
        assert!(book.builtin_names.contains("print_area"));
        assert!(!book.builtin_names.contains("secret"));
    }

    #[test]
    fn truncated_tokens_end_the_walk() {
        assert_eq!(check_tokens(&[0x17, 9], W), Ok(0));
        assert_eq!(check_tokens(&[0x24, 1, 2], W), Ok(16));
        assert_eq!(check_tokens(&[0xFF], W), Ok(0));
    }

    #[test]
    fn record_past_the_end_is_an_error() {
        let bytes = [0x05, 0xFF, 0xFF, 0xFF, 0x7F];
        let mut r = Records::new(&bytes);
        assert!(r.next().unwrap().is_err());
    }
}
