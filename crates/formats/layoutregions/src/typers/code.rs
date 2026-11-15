// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The typer of codes: a barcode or a matrix code the reader found, and the line that
//! prints its value for people under or over it.
//!
//! The segmentation gives each code a leaf of its own (see
//! [`crate::segment::segment_page`]), and its human-readable line stands in a text segment
//! beside it, so the line is looked for on the page by its place. A [`crate::CodeIn`] holds
//! the code's box only, so the line is matched by its place and its shape, not by the value.

use crate::input::PrimId;
use crate::region::{Bounds, Kind};
use crate::segment::Segment;
use crate::typers::figure::has_ink;
use crate::typers::{PageView, Typed, signal};

/// `code.symbol`: the segment holds a code and no text with ink or picture.
pub const CODE_SYMBOL: u16 = 700;
/// `code.readable_line`: a line in the shape of a code's value stands right under or over a
/// code of the segment, within its width.
pub const CODE_READABLE: u16 = 150;

/// How far a human-readable line may stand from its code: this many of its own text sizes.
const READABLE_GAP: f32 = 2.0;
/// The most words a human-readable line has.
const READABLE_WORDS: usize = 4;

/// Whether `text` has the shape of a code's value: at most [`READABLE_WORDS`] words of ASCII
/// letters, digits and `-./*+:`, with a digit or a word of capitals only (two letters or
/// more), so a word such as "Muster" is not a code's value.
pub(crate) fn readable_shape(text: &str) -> bool {
    let words = text.split_whitespace().count();
    (1..=READABLE_WORDS).contains(&words)
        && text
            .chars()
            .filter(|c| !c.is_whitespace())
            .all(|c| c.is_ascii_alphanumeric() || "-./*+:".contains(c))
        && (text.chars().any(|c| c.is_ascii_digit())
            || text.split_whitespace().any(|w| {
                w.chars().filter(|c| c.is_ascii_alphabetic()).count() >= 2
                    && !w.chars().any(|c| c.is_ascii_lowercase())
            }))
}

/// Whether a line of the page of `view` stands under or over the code box `b`: its gap to the
/// box at most [`READABLE_GAP`] of its text size, its runs within the box widened by one text
/// size on each side, and its text in the shape of a code's value (see [`readable_shape`]).
fn readable_line(view: &PageView<'_>, b: &Bounds) -> bool {
    let texts = &view.page.texts;
    let by_y = view.by_y();
    let lines = view.lines();
    // A line of the largest size a caption search assumes can stand this far off.
    let reach = READABLE_GAP * crate::extract::table::caption::MAX_CAPTION_SIZE;
    let lo = by_y.line_y.partition_point(|y| *y < b.y0 - 2.0 * reach);
    let hi = by_y.line_y.partition_point(|y| *y <= b.y1 + reach);
    crate::work::add((hi - lo) as u64);
    by_y.lines[lo..hi].iter().any(|&k| {
        let l = &lines[k];
        let runs = &texts[l.start..l.end];
        if runs.iter().any(|r| r.off_axis) {
            return false;
        }
        let s = l.size.max(1.0);
        let gap_under = b.y0 - (l.y + s);
        let gap_over = l.y - b.y1;
        let near = (-0.5 * s..=READABLE_GAP * s).contains(&gap_under)
            || (-0.5 * s..=READABLE_GAP * s).contains(&gap_over);
        if !near {
            return false;
        }
        let x0 = runs
            .iter()
            .map(|r| r.x.min(r.x + r.w))
            .fold(f32::MAX, f32::min);
        let x1 = runs
            .iter()
            .map(|r| r.x.max(r.x + r.w))
            .fold(f32::MIN, f32::max);
        if x0 < b.x0 - s || x1 > b.x1 + s {
            return false;
        }
        let text: Vec<&str> = runs.iter().map(|r| r.text.trim()).collect();
        crate::work::add(runs.len() as u64);
        readable_shape(&text.join(" "))
    })
}

/// Code: the segment holds a code and no text with ink or picture (`code.symbol`; a segment of
/// a code and a picture is the parent of both, each a leaf of its own), and a line in the
/// shape of a code's value stands right under or over one of its codes (`code.readable_line`).
pub fn type_code(view: &PageView<'_>, seg: &Segment) -> Option<Typed> {
    view.code_of(seg)
}

/// [`type_code`], worked out.
pub(crate) fn code(view: &PageView<'_>, seg: &Segment) -> Option<Typed> {
    crate::work::add(1 + seg.prims.len() as u64);
    let codes: Vec<&Bounds> = seg
        .prims
        .iter()
        .filter_map(|p| match *p {
            PrimId::Code(i) => view.page.codes.get(i as usize).map(|c| &c.bounds),
            _ => None,
        })
        .collect();
    let pictures = seg.prims.iter().any(|p| matches!(p, PrimId::Picture(_)));
    if codes.is_empty() || pictures || has_ink(view.page, seg) {
        return None;
    }
    let mut signals = vec![signal(
        "code.symbol",
        format!("{} codes", codes.len()),
        CODE_SYMBOL,
    )];
    if codes.iter().any(|b| readable_line(view, b)) {
        signals.push(signal(
            "code.readable_line",
            "a line of its value stands next to it".into(),
            CODE_READABLE,
        ));
    }
    Typed::of(Kind::Code, signals)
}
