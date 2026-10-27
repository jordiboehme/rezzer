// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The rule that makes an output name legal on Windows (Jordi, 2026-10-27).
//!
//! Windows cannot write every name that Unix and macOS can: a `:` writes an NTFS alternate
//! data stream into a file of that name, `< > " | ? *` and the ASCII control characters are
//! refused, a trailing dot or space is dropped, and a name whose part before the first `.`
//! is a device name (`CON`, `PRN`, `AUX`, `NUL`, `COM1` to `COM9`, `LPT1` to `LPT9`, in any
//! case, also with spaces before the `.`) opens the device instead of a file. So when Rezzer
//! writes on Windows, a name it derives from a source name gets each such character, each
//! trailing dot or space and such a device name replaced by `_` ([`legal_name`]).
//! [`crate::files`] then adds the name hash of the original name, so two sources never share
//! an output name. On Unix and macOS the names stay as they are.
//!
//! The OS is a parameter ([`CacheOs`]), so the rule is the same pure function on every host
//! and its Windows branch is tested on Unix too.

use std::borrow::Cow;

use crate::config::CacheOs;

/// The device names Windows matches before the first `.` of a name, besides `COM` and `LPT`
/// with a digit.
const DEVICES: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];

/// Whether `c` is a character NTFS refuses in a name: `: < > " | ? *` and 0x00 to 0x1F (the
/// separators `/` and `\` never reach a name).
fn forbidden(c: char) -> bool {
    matches!(c, ':' | '<' | '>' | '"' | '|' | '?' | '*') || c < '\u{20}'
}

/// Whether `base` is a device name: `CON`, `PRN`, `AUX`, `NUL`, or `COM` or `LPT` with one
/// digit 1 to 9 or one superscript 1 to 3 (Windows reserves `COM¹` too), in any case.
fn device(base: &str) -> bool {
    if DEVICES.iter().any(|d| base.eq_ignore_ascii_case(d)) {
        return true;
    }
    let (Some(head), Some(tail)) = (base.get(..3), base.get(3..)) else {
        return false;
    };
    let port = head.eq_ignore_ascii_case("COM") || head.eq_ignore_ascii_case("LPT");
    let mut chars = tail.chars();
    let number = matches!(
        (chars.next(), chars.next()),
        (Some('1'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}'), None)
    );
    port && number
}

/// `name` as it may be written on `os`. On Windows every forbidden character, every
/// trailing dot or space, and a device name before the first `.` (trailing spaces there
/// count with it) become `_`; the result is [`Cow::Owned`] exactly when something changed.
/// On Unix and macOS `name` is returned as it is.
pub(crate) fn legal_name(name: &str, os: CacheOs) -> Cow<'_, str> {
    if os != CacheOs::Windows {
        return Cow::Borrowed(name);
    }
    let mut out: String = name
        .chars()
        .map(|c| if forbidden(c) { '_' } else { c })
        .collect();
    let kept = out.trim_end_matches(['.', ' ']).len();
    let trailing = out.len() - kept;
    out.truncate(kept);
    out.extend(std::iter::repeat_n('_', trailing));
    let base_end = out.find('.').unwrap_or(out.len());
    let base = out[..base_end].trim_end_matches(' ');
    if device(base) {
        out.replace_range(..base.len(), "_");
    }
    if out == name {
        Cow::Borrowed(name)
    } else {
        Cow::Owned(out)
    }
}
