// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Links, wikilink targets, math and references in inline text.

use super::*;

pub(super) fn inline_math(latex: &str, mode: Mode, out: &mut String) {
    if latex.trim().is_empty() {
        return;
    }
    let content = one_line(latex, mode);
    if !content.contains('`') {
        out.push_str("$`");
        out.push_str(&content);
        out.push_str("`$");
    } else if !content.contains('$') && !content.contains('<') {
        out.push_str("$$");
        out.push_str(&content);
        out.push_str("$$");
    } else {
        // Neither math form can hold it; it stays readable as code.
        code_span(&content, mode, out);
    }
}

/// The schemes a link from a document may have: addresses come from the file and are not
/// trusted, so `file:`, `ftp:`, `data:` and script schemes stay text.
const LINK_SCHEMES: [&str; 3] = ["http", "https", "mailto"];

/// Whether `scheme` is one of [`LINK_SCHEMES`], in any case.
pub(super) fn link_scheme(scheme: &str) -> bool {
    LINK_SCHEMES.iter().any(|s| scheme.eq_ignore_ascii_case(s))
}

/// True for a target with no scheme (a relative path or `#anchor`) or with one of
/// [`LINK_SCHEMES`]. Tabs and newlines are ignored and leading spaces and controls
/// trimmed first, as browsers do.
pub(super) fn safe_target(target: &str) -> bool {
    let decoded = decode_references(target);
    let cleaned: String = decoded
        .trim_start_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    // With a `:` anywhere, an `&` before the first one could hide a scheme: unsafe. This is
    // checked on the target as written and as decoded.
    let raw: String = target
        .trim_start_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    for text in [&raw, &cleaned] {
        if let Some(colon) = text.find(':')
            && text[..colon].contains('&')
        {
            return false;
        }
    }
    let before = cleaned.split(['/', '?', '#']).next().unwrap_or("");
    let Some((scheme, _)) = before.split_once(':') else {
        return true;
    };
    let is_scheme = scheme.len() >= 2
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !is_scheme {
        return true;
    }
    link_scheme(scheme)
}

/// A numeric reference after `&#`: decimal or `x` hex digits of any length, with or
/// without the closing `;`, as browsers read them in attributes. Returns the character and
/// the bytes used.
fn numeric_reference(text: &str) -> Option<(char, usize)> {
    let (digits_start, radix) = match text.as_bytes().first() {
        Some(b'x' | b'X') => (1, 16),
        _ => (0, 10),
    };
    let digits = text[digits_start..]
        .chars()
        .take_while(|c| c.is_digit(radix))
        .count();
    if digits == 0 {
        return None;
    }
    let mut value: u32 = 0;
    for c in text[digits_start..digits_start + digits].chars() {
        // Saturates far above the last code point, so long zero runs and huge values work.
        value = value
            .saturating_mul(radix)
            .saturating_add(c.to_digit(radix).unwrap_or(0))
            .min(0x11_0000);
    }
    let c = char::from_u32(value)
        .filter(|c| *c != '\0')
        .unwrap_or('\u{fffd}');
    let mut used = digits_start + digits;
    if text[used..].starts_with(';') {
        used += 1;
    }
    Some((c, used))
}

/// Decodes numeric character references and the named ones that could spell a scheme or
/// its separators (`&colon;`, `&Tab;`, ...), as a Markdown reader would before a browser
/// sees the target.
fn decode_references(text: &str) -> String {
    const NAMED: &[(&str, char)] = &[
        ("colon", ':'),
        ("Tab", '\t'),
        ("NewLine", '\n'),
        ("sol", '/'),
        ("quest", '?'),
        ("num", '#'),
        ("period", '.'),
        ("comma", ','),
        ("plus", '+'),
        ("hyphen", '-'),
        ("dash", '-'),
        ("minus", '-'),
        ("lpar", '('),
        ("rpar", ')'),
        ("semi", ';'),
        ("excl", '!'),
        ("equals", '='),
        ("commat", '@'),
        ("percnt", '%'),
        ("amp", '&'),
        ("AMP", '&'),
        ("lt", '<'),
        ("LT", '<'),
        ("gt", '>'),
        ("GT", '>'),
        ("quot", '"'),
        ("QUOT", '"'),
        ("apos", '\''),
        ("nbsp", ' '),
    ];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 1..];
        let decoded = if let Some(num) = after.strip_prefix('#') {
            numeric_reference(num).map(|(c, len)| (c, len + 1))
        } else {
            // Named references need their `;`.
            after.find(';').filter(|&end| end <= 32).and_then(|end| {
                let name = &after[..end];
                NAMED
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, c)| (*c, end + 1))
            })
        };
        match decoded {
            Some((c, used)) => {
                out.push(c);
                rest = &after[used..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Writes one character of a link destination. Its size must stay within
/// `rezzer_model::link_target_written_len`.
pub(super) fn destination_char(c: char, out: &mut String) {
    match c {
        ' ' => out.push_str("%20"),
        '(' => out.push_str("%28"),
        ')' => out.push_str("%29"),
        '<' => out.push_str("%3C"),
        '>' => out.push_str("%3E"),
        '[' => out.push_str("%5B"),
        ']' => out.push_str("%5D"),
        '|' => out.push_str("%7C"),
        '&' => out.push_str("\\&"),
        '\\' => out.push_str("\\\\"),
        c if c.is_control() => {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        c => out.push(c),
    }
}

/// The path of an extracted file as a link destination: `%` and `#` are encoded first, so a
/// reader that decodes percent codes or cuts the target at `#` still finds the file. The
/// rest is encoded as in any destination. Only asset paths take this: a web address keeps
/// its percent codes and an anchor its `#`.
pub(crate) fn asset_target(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        match c {
            '%' => out.push_str("%25"),
            '#' => out.push_str("%23"),
            c => out.push(c),
        }
    }
    out
}

/// A link to an extracted file, its path as the text; `base` goes before the path in the
/// target (see [`Ctx::link_base`]).
pub(crate) fn asset_link(path: &str, base: &str, mode: Mode) -> String {
    inlines(
        &[Inline::Link {
            target: format!("{base}{}", asset_target(path)),
            title: None,
            internal: false,
            wiki: false,
            inlines: vec![Inline::plain(path)],
        }],
        mode,
    )
}

/// A link or image destination plus optional title, without the parentheses.
pub(super) fn destination(target: &str, title: Option<&str>, mode: Mode, out: &mut String) {
    if target.is_empty() {
        out.push_str("<>");
    }
    for c in target.chars() {
        destination_char(c, out);
    }
    if let Some(title) = title
        && !title.is_empty()
    {
        out.push_str(" \"");
        for c in title.chars() {
            match c {
                '"' | '\\' => {
                    out.push('\\');
                    out.push(c);
                }
                '\n' | '\r' => out.push(' '),
                '<' => out.push_str("&lt;"),
                '|' if mode == Mode::Cell => out.push_str("\\|"),
                '[' | ']' => {
                    out.push('\\');
                    out.push(c);
                }
                c => out.push(c),
            }
        }
        out.push('"');
    }
}

/// A footnote label without characters that would end or break `[^...]`.
pub(super) fn footnote_label(id: &str, mode: Mode) -> String {
    let mut label = String::new();
    for c in id.chars() {
        if c.is_whitespace() || matches!(c, '[' | ']' | '\\' | '^' | '<' | '>') {
            continue;
        }
        if c == '|' && mode == Mode::Cell {
            label.push('\\');
        }
        label.push(c);
    }
    if label.is_empty() {
        "note".into()
    } else {
        label
    }
}
