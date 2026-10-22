// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The picture sizes image codecs state in their own headers, read without decoding: a
//! decoder sizes its buffers from them.

/// Width, height and components of a JPEG frame (its SOF marker), and whether it is not a
/// baseline or extended sequential frame.
pub(crate) fn jpeg_frame(data: &[u8]) -> Option<(u64, u64, u64, bool)> {
    let mut i = 2;
    if data.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        match marker {
            0xFF => i += 1,
            0x01 | 0xD0..=0xD8 => i += 2,
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                let h = u16::from_be_bytes([*data.get(i + 5)?, *data.get(i + 6)?]);
                let w = u16::from_be_bytes([*data.get(i + 7)?, *data.get(i + 8)?]);
                let c = *data.get(i + 9)?;
                let sequential = matches!(marker, 0xC0 | 0xC1);
                return Some((u64::from(w), u64::from(h), u64::from(c), !sequential));
            }
            _ => {
                let len = u16::from_be_bytes([*data.get(i + 2)?, *data.get(i + 3)?]);
                i += 2 + usize::from(len);
            }
        }
    }
    None
}

/// Width, height and components of a JPEG 2000 picture: the `ihdr` box of a JP2 file, or the
/// SIZ marker of a bare codestream.
pub(crate) fn jpx_frame(data: &[u8]) -> Option<(u64, u64, u64)> {
    let u32_at = |i: usize| -> Option<u64> {
        Some(u64::from(u32::from_be_bytes(
            data.get(i..i + 4)?.try_into().ok()?,
        )))
    };
    let head = &data[..data.len().min(1 << 16)];
    if let Some(at) = head.windows(4).position(|w| w == b"ihdr") {
        let h = u32_at(at + 4)?;
        let w = u32_at(at + 8)?;
        let c = u64::from(u16::from_be_bytes(
            data.get(at + 12..at + 14)?.try_into().ok()?,
        ));
        return Some((w, h, c));
    }
    let at = head.windows(2).position(|w| w == [0xFF, 0x51])?;
    // Lsiz, Rsiz, Xsiz, Ysiz, XOsiz, YOsiz, then tiles, then Csiz.
    let x = u32_at(at + 6)?;
    let y = u32_at(at + 10)?;
    let xo = u32_at(at + 14)?;
    let yo = u32_at(at + 18)?;
    let c = u64::from(u16::from_be_bytes(
        data.get(at + 38..at + 40)?.try_into().ok()?,
    ));
    Some((x.saturating_sub(xo), y.saturating_sub(yo), c))
}

/// Width and height of the page of an embedded JBIG2 stream (its page information segment).
pub(crate) fn jbig2_page(data: &[u8]) -> Option<(u64, u64)> {
    let mut i = 0usize;
    for _ in 0..64 {
        let number = u32::from_be_bytes(data.get(i..i + 4)?.try_into().ok()?);
        let flags = *data.get(i + 4)?;
        let kind = flags & 0x3F;
        let refs = *data.get(i + 5)? >> 5;
        if refs > 4 {
            return None;
        }
        let ref_size = if number <= 256 {
            1
        } else if number <= 65_536 {
            2
        } else {
            4
        };
        let page_size = if flags & 0x40 != 0 { 4 } else { 1 };
        let mut at = i + 6 + usize::from(refs) * ref_size + page_size;
        let len = u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?);
        at += 4;
        if kind == 48 {
            let w = u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?);
            let h = u32::from_be_bytes(data.get(at + 4..at + 8)?.try_into().ok()?);
            if h == u32::MAX {
                return None;
            }
            return Some((u64::from(w), u64::from(h)));
        }
        if len == u32::MAX {
            return None;
        }
        i = at.checked_add(usize::try_from(len).ok()?)?;
    }
    None
}
