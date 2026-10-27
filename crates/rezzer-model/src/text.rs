// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Small text helpers that readers and the renderer must apply the same way: a heading's
//! slug, a link target's percent decoding and text on one line.

/// A heading id the way GitHub builds it: inline HTML tags left out, lowercase letters and
/// digits, a space or `-` as `-`, `_` kept, other punctuation dropped. The Markdown reader
/// names its heading anchors with it, and the splitter the fragments of links into a part.
pub fn heading_slug(text: &str) -> String {
    let mut slug = String::new();
    for c in strip_html_tags(text).trim().chars() {
        if c.is_alphanumeric() {
            slug.extend(c.to_lowercase());
        } else if c == ' ' || c == '-' {
            slug.push('-');
        } else if c == '_' {
            slug.push('_');
        }
    }
    slug
}

/// `text` without HTML tags: a `<` followed by a letter, `/` or `!` up to the next `>`.
/// Any other `<` stays.
pub fn strip_html_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let is_tag = tail[1..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '/' || c == '!');
        match tail.find('>') {
            Some(j) if is_tag => rest = &tail[j + 1..],
            _ => {
                out.push('<');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `%XX` escapes decoded to their bytes, read as UTF-8 (a broken sequence becomes U+FFFD).
/// A `%` without two hex digits stays.
pub fn percent_decode(text: &str) -> String {
    if !text.contains('%') {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(hi), Some(lo)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            )
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Text on one line: every run of white space and control characters becomes one space, and
/// none is left at the ends.
pub fn one_line(text: &str) -> String {
    text.split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Text with every control character (line breaks included) as a space, nothing else
/// changed: the length in characters stays.
pub fn controls_to_spaces(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Bytes as lowercase hexadecimal, two digits a byte, in a string made at its size.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_leave_out_inline_html() {
        assert_eq!(heading_slug("Beta <b>bold</b>"), "beta-bold");
        assert_eq!(heading_slug(" A_b, c-d! "), "a_b-c-d");
        assert_eq!(heading_slug("1 < 2"), "1--2");
        assert_eq!(heading_slug("Über"), "über");
    }

    #[test]
    fn percent_escapes_decode() {
        assert_eq!(percent_decode("a%20b%zz%"), "a b%zz%");
        assert_eq!(percent_decode("caf%C3%A9"), "café");
        assert_eq!(percent_decode("%FF"), "\u{fffd}");
    }

    #[test]
    fn text_on_one_line() {
        assert_eq!(one_line(" a \n\t b\u{1}c  "), "a b c");
        assert_eq!(controls_to_spaces("a\nb\u{1}"), "a b ");
    }
}
