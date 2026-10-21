// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! PNG and TIFF predictors (ISO 32000-1, 7.4.4.4), which most cross-reference streams and
//! many object streams use after Flate. The output is never longer than the input.

use lopdf::{Dictionary, Object};

/// The decode parameters of the last filter of a stream, direct values only.
pub(crate) fn last_parms(dict: &Dictionary) -> Option<&Dictionary> {
    match dict.get(b"DecodeParms").ok()? {
        Object::Dictionary(d) => Some(d),
        Object::Array(a) => a.iter().rev().find_map(|o| o.as_dict().ok()),
        _ => None,
    }
}

fn int(d: &Dictionary, key: &[u8], default: i64) -> i64 {
    d.get(key)
        .ok()
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(default)
}

/// `data` cut to whole rows of the predictor named in `parms`: a row cut off by damage
/// cannot be undone.
pub(crate) fn whole_rows(mut data: Vec<u8>, parms: Option<&Dictionary>) -> Vec<u8> {
    let Some(p) = parms else {
        return data;
    };
    let predictor = int(p, b"Predictor", 1);
    let colors = int(p, b"Colors", 1).clamp(1, 32) as usize;
    let bpc = int(p, b"BitsPerComponent", 8).clamp(1, 16) as usize;
    let columns = int(p, b"Columns", 1).clamp(1, 1 << 24) as usize;
    let row = (colors * bpc * columns).div_ceil(8);
    // PNG rows start with a byte that names their filter.
    let stride = match predictor {
        10..=15 => row + 1,
        2 => row,
        _ => return data,
    };
    data.truncate(data.len() - data.len() % stride.max(1));
    data
}

/// Undoes the predictor named in `parms`, if any.
pub(crate) fn unpredict(data: Vec<u8>, parms: Option<&Dictionary>) -> Vec<u8> {
    let Some(p) = parms else {
        return data;
    };
    let predictor = int(p, b"Predictor", 1);
    let colors = int(p, b"Colors", 1).clamp(1, 32) as usize;
    let bpc = int(p, b"BitsPerComponent", 8).clamp(1, 16) as usize;
    let columns = int(p, b"Columns", 1).clamp(1, 1 << 24) as usize;
    let bpp = (colors * bpc).div_ceil(8).max(1);
    let row = (colors * bpc * columns).div_ceil(8);
    match predictor {
        2 if bpc == 8 => tiff(data, row, bpp),
        10..=15 => png(&data, row, bpp),
        _ => data,
    }
}

fn tiff(mut data: Vec<u8>, row: usize, bpp: usize) -> Vec<u8> {
    for line in data.chunks_mut(row) {
        for i in bpp..line.len() {
            line[i] = line[i].wrapping_add(line[i - bpp]);
        }
    }
    data
}

fn png(data: &[u8], row: usize, bpp: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    // A row is never longer than the data, whatever /Columns says.
    let mut prev = vec![0u8; row.min(data.len())];
    for chunk in data.chunks(row + 1) {
        let kind = chunk[0];
        let cur = &chunk[1..];
        let mut line = vec![0u8; cur.len()];
        for i in 0..cur.len() {
            let left = if i >= bpp { line[i - bpp] } else { 0 };
            let up = prev[i];
            let up_left = if i >= bpp { prev[i - bpp] } else { 0 };
            let add = match kind {
                1 => left,
                2 => up,
                3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
                4 => paeth(left, up, up_left),
                _ => 0,
            };
            line[i] = cur[i].wrapping_add(add);
        }
        out.extend_from_slice(&line);
        prev[..line.len()].copy_from_slice(&line);
    }
    out
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let (pa, pb, pc) = (
        (p - i16::from(a)).abs(),
        (p - i16::from(b)).abs(),
        (p - i16::from(c)).abs(),
    );
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    #[test]
    fn png_up_rows_are_added() {
        let parms = dictionary! { "Predictor" => 12, "Columns" => 3 };
        // Row 1 with filter None, row 2 with filter Up.
        let data = vec![0, 1, 2, 3, 2, 1, 1, 1];
        assert_eq!(unpredict(data, Some(&parms)), vec![1, 2, 3, 2, 3, 4]);
    }

    #[test]
    fn huge_columns_cost_no_more_than_the_data() {
        let parms = dictionary! { "Predictor" => 12, "Columns" => 1 << 24, "Colors" => 32, "BitsPerComponent" => 16 };
        assert_eq!(unpredict(vec![2, 7, 7], Some(&parms)), vec![7, 7]);
    }

    #[test]
    fn no_predictor_keeps_data() {
        assert_eq!(unpredict(vec![1, 2], None), vec![1, 2]);
    }
}
