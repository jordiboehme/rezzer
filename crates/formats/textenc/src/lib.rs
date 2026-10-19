// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Turns bytes in any text encoding into UTF-8.
//!
//! [`decode`] tries, in order: a byte order mark (UTF-8, UTF-16LE, UTF-16BE), strict UTF-8,
//! then statistical detection with `chardetng` and decoding with `encoding_rs`. It never
//! fails: undecodable bytes become U+FFFD and the result is flagged as not confident.
//!
//! The crate has no dependency on the rest of Rezzer.

use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
use encoding_rs::{Encoding, ISO_2022_JP, UTF_8};

/// Inputs up to this size are shown whole to the statistical detector.
const DETECT_BYTES: usize = 64 * 1024 * 1024;

/// Size of each sample (start, middle, end) of a larger input.
const SAMPLE_BYTES: usize = 8 * 1024 * 1024;

/// The line ending style found in a text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    /// Only `\n`.
    Lf,
    /// Only `\r\n`.
    CrLf,
    /// Only a lone `\r`.
    Cr,
    /// More than one style.
    Mixed,
    /// No line break at all.
    None,
}

/// The result of [`decode`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// The text as UTF-8, without a byte order mark.
    pub text: String,
    /// WHATWG name of the detected encoding, for example `UTF-8`, `windows-1252`,
    /// `Shift_JIS` or `UTF-16LE`.
    pub encoding: &'static str,
    /// True when the input started with a byte order mark.
    pub had_bom: bool,
    /// False when bytes could not be decoded and were replaced with U+FFFD.
    pub confident: bool,
    /// The line ending style of `text` (the original is not changed).
    pub line_ending: LineEnding,
}

/// Decodes `bytes` of unknown encoding into UTF-8 text.
///
/// Order: byte order mark, strict UTF-8, then `chardetng` (no top-level domain hint, UTF-8 not
/// allowed as a guess because strict UTF-8 already failed). Input that is valid UTF-8 but holds
/// ISO-2022-JP escape sequences is decoded as ISO-2022-JP. Inputs up to 64 MiB are shown whole to
/// the detector, larger ones are sampled (start, middle, end); the whole input is decoded.
pub fn decode(bytes: &[u8]) -> Decoded {
    if let Some((enc, bom_len)) = Encoding::for_bom(bytes) {
        return finish(enc, &bytes[bom_len..], true);
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        // ISO-2022-JP is 7-bit, so it is valid UTF-8; its escape sequences give it away.
        let iso = bytes.contains(&0x1B) && guess(bytes) == ISO_2022_JP;
        if !iso {
            return Decoded {
                line_ending: line_ending(text),
                text: text.to_owned(),
                encoding: UTF_8.name(),
                had_bom: false,
                confident: true,
            };
        }
    }
    finish(guess(bytes), bytes, false)
}

/// Asks the statistical detector. The detector is streaming and linear, so inputs up to
/// 64 MiB are fed whole; larger ones as 8 MiB from the start, the middle and the end.
fn guess(bytes: &[u8]) -> &'static Encoding {
    let mut detector = EncodingDetector::new(Iso2022JpDetection::Allow);
    if bytes.len() <= DETECT_BYTES {
        detector.feed(bytes, true);
    } else {
        let len = bytes.len();
        let mid = len / 2 - SAMPLE_BYTES / 2;
        detector.feed(&bytes[..SAMPLE_BYTES], false);
        detector.feed(&bytes[mid..mid + SAMPLE_BYTES], false);
        detector.feed(&bytes[len - SAMPLE_BYTES..], true);
    }
    detector.guess(None, Utf8Detection::Deny)
}

fn finish(enc: &'static Encoding, bytes: &[u8], had_bom: bool) -> Decoded {
    let (text, replaced) = {
        let (cow, replaced) = enc.decode_without_bom_handling(bytes);
        (cow.into_owned(), replaced)
    };
    Decoded {
        line_ending: line_ending(&text),
        text,
        encoding: enc.name(),
        had_bom,
        confident: !replaced,
    }
}

fn line_ending(text: &str) -> LineEnding {
    let (mut lf, mut crlf, mut cr) = (false, false, false);
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\r' if b.get(i + 1) == Some(&b'\n') => {
                crlf = true;
                i += 1;
            }
            b'\r' => cr = true,
            b'\n' => lf = true,
            _ => {}
        }
        i += 1;
    }
    match (lf, crlf, cr) {
        (false, false, false) => LineEnding::None,
        (true, false, false) => LineEnding::Lf,
        (false, true, false) => LineEnding::CrLf,
        (false, false, true) => LineEnding::Cr,
        _ => LineEnding::Mixed,
    }
}
