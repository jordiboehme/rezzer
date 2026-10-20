// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Body blocks and inlines as Markdown in Crystalline's dialect.
//!
//! Text from files is escaped so that it can never add structure: no headings, lists, quotes,
//! tables, code, HTML (and so no HTML comments), math or frontmatter. Anchors are not printed:
//! the model JSON keeps them, and a heading anchor is the heading's own slug anyway.

use rezzer_model::{Align, Asset, Block, CalloutKind, Cell, Inline, StatusColor, TaskState};

/// Deeper nesting than this is cut with a visible note, so a hostile file cannot overflow
/// the stack.
pub(crate) const MAX_DEPTH: usize = 64;

const TOO_DEEP: &str = "Nesting too deep: the rest of this part is not shown.";

/// Where inline text goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// A paragraph: newlines stay.
    Block,
    /// One line: a heading, a title, link text.
    Line,
    /// One line inside a table cell: `|` is escaped even inside code.
    Cell,
}

/// What the block renderer needs besides the blocks.
pub(crate) struct Ctx<'a> {
    pub assets: &'a [Asset],
    /// Added to every body heading level, so body headings stay below the title.
    pub heading_offset: u8,
}

// ---------------------------------------------------------------------------------------
// Inlines

/// Renders inlines. In [`Mode::Block`] the result still needs [`finish_lines`].
pub(crate) fn inlines(list: &[Inline], mode: Mode) -> String {
    let merged = fix_flanking(merge_text(list));
    let dollars = count_dollars(&merged) >= 2;
    let mut out = String::new();
    for inline in &merged {
        inline_into(inline, mode, 0, dollars, &mut out);
    }
    out
}

/// The `$` signs in the text runs of `list` (not in code), each math span counted as two,
/// walked with a stack.
fn count_dollars(list: &[Inline]) -> usize {
    let mut n = 0;
    let mut stack: Vec<std::slice::Iter<'_, Inline>> = vec![list.iter()];
    while let Some(top) = stack.last_mut() {
        let Some(inline) = top.next() else {
            stack.pop();
            continue;
        };
        match inline {
            Inline::Text { text, .. }
            | Inline::Status { text, .. }
            | Inline::Image { alt: text, .. } => n += text.matches('$').count(),
            Inline::Link { inlines, .. } => stack.push(inlines.iter()),
            // A math span writes two `$` that a text `$` could pair with.
            Inline::Math { latex } if !latex.trim().is_empty() => n += 2,
            _ => {}
        }
    }
    n
}

/// Joins neighbouring text runs with the same style, so `**a****b**` never happens.
fn merge_text(list: &[Inline]) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(list.len());
    for inline in list {
        if let (
            Some(Inline::Text {
                text: last,
                emphasis: e1,
                strong: s1,
                strike: k1,
            }),
            Inline::Text {
                text,
                emphasis,
                strong,
                strike,
            },
        ) = (out.last_mut(), inline)
            && *e1 == *emphasis
            && *s1 == *strong
            && *k1 == *strike
        {
            last.push_str(text);
            continue;
        }
        out.push(inline.clone());
    }
    out
}

fn is_styled(inline: &Inline) -> bool {
    matches!(
        inline,
        Inline::Text { emphasis, strong, strike, .. } if *emphasis || *strong || *strike
    )
}

/// Punctuation in CommonMark's sense (Unicode P and S), approximated as neither letter,
/// digit nor whitespace.
fn is_punct(c: char) -> bool {
    !c.is_alphanumeric() && !c.is_whitespace()
}

/// The first (`first`) or last character an inline puts into the Markdown. `None` for an
/// inline that writes nothing.
fn edge_char(inline: &Inline, first: bool) -> Option<char> {
    let pick = |a: char, b: char| Some(if first { a } else { b });
    match inline {
        Inline::Text { text, .. } => {
            let c = if first {
                text.chars().next()
            } else {
                text.chars().next_back()
            }?;
            if is_styled(inline) && !c.is_whitespace() {
                // The delimiter.
                Some('*')
            } else {
                Some(c)
            }
        }
        Inline::LineBreak => Some('\n'),
        Inline::Image { .. } => pick('!', ')'),
        Inline::Code { text } if !text.is_empty() => Some('`'),
        Inline::Math { latex } if !latex.trim().is_empty() => Some('$'),
        Inline::Link { .. } => pick('[', ')'),
        Inline::FootnoteRef { .. } => pick('[', ']'),
        Inline::Status { .. } => pick(':', '}'),
        Inline::Code { .. } | Inline::Math { .. } | Inline::AnchorMark { .. } => None,
    }
}

/// The character before inline `i` (`None` at the start).
fn char_before(list: &[Inline], i: usize) -> Option<char> {
    list[..i].iter().rev().find_map(|x| edge_char(x, false))
}

fn char_after(list: &[Inline], i: usize) -> Option<char> {
    list[i + 1..].iter().find_map(|x| edge_char(x, true))
}

/// Whether the delimiters around a styled run would open and close (CommonMark's left- and
/// right-flanking rules; `*` and `~~` also work inside words).
fn flanks(list: &[Inline], i: usize) -> (bool, bool) {
    let Inline::Text { text, .. } = &list[i] else {
        return (true, true);
    };
    let core = text.trim();
    let (Some(first), Some(last)) = (core.chars().next(), core.chars().next_back()) else {
        return (true, true);
    };
    let before = if text.starts_with(char::is_whitespace) {
        Some(' ')
    } else {
        char_before(list, i)
    };
    let after = if text.ends_with(char::is_whitespace) {
        Some(' ')
    } else {
        char_after(list, i)
    };
    let free = |c: Option<char>| c.is_none_or(|c| c.is_whitespace() || is_punct(c));
    let opens = !is_punct(first) || free(before);
    let closes = !is_punct(last) || free(after);
    (opens, closes)
}

/// Makes every styled run parse: where a delimiter would not open or close, the run takes
/// in the rest of the word from the plain text next to it; where that does not help, the
/// style is dropped and the text stays.
fn fix_flanking(mut list: Vec<Inline>) -> Vec<Inline> {
    for i in 0..list.len() {
        if !is_styled(&list[i]) {
            continue;
        }
        let (opens, closes) = flanks(&list, i);
        if !opens
            && i > 0
            && let Some(word) = take_word(&mut list, i - 1, false)
            && let Inline::Text { text, .. } = &mut list[i]
        {
            text.insert_str(0, &word);
        }
        if !closes
            && i + 1 < list.len()
            && let Some(word) = take_word(&mut list, i + 1, true)
            && let Inline::Text { text, .. } = &mut list[i]
        {
            text.push_str(&word);
        }
        let (opens, closes) = flanks(&list, i);
        if !(opens && closes)
            && let Inline::Text {
                emphasis,
                strong,
                strike,
                ..
            } = &mut list[i]
        {
            *emphasis = false;
            *strong = false;
            *strike = false;
        }
    }
    list
}

/// Takes the word at the start (`leading`) or end of plain text inline `j`.
fn take_word(list: &mut [Inline], j: usize, leading: bool) -> Option<String> {
    if is_styled(&list[j]) {
        return None;
    }
    let Inline::Text { text, .. } = &mut list[j] else {
        return None;
    };
    if leading {
        let end = text.find(char::is_whitespace).unwrap_or(text.len());
        if end == 0 {
            return None;
        }
        let word = text[..end].to_string();
        text.replace_range(..end, "");
        Some(word)
    } else {
        let start = text.rfind(char::is_whitespace).map_or(0, |p| {
            p + text[p..].chars().next().map_or(1, char::len_utf8)
        });
        if start == text.len() {
            return None;
        }
        let word = text[start..].to_string();
        text.truncate(start);
        Some(word)
    }
}

fn inline_into(inline: &Inline, mode: Mode, link_depth: usize, dollars: bool, out: &mut String) {
    match inline {
        Inline::Text {
            text,
            emphasis,
            strong,
            strike,
        } => styled_text(text, *emphasis, *strong, *strike, mode, dollars, out),
        Inline::LineBreak => {
            if mode == Mode::Block {
                out.push_str("\\\n");
            } else {
                out.push(' ');
            }
        }
        Inline::Image { target, alt, title } => {
            if !safe_target(target) {
                escape_with(alt, line_mode(mode), dollars, out);
                out.push(' ');
                code_span(target, line_mode(mode), out);
                return;
            }
            out.push_str("![");
            escape_with(alt, line_mode(mode), dollars, out);
            out.push_str("](");
            destination(target, title.as_deref(), mode, out);
            out.push(')');
        }
        Inline::Code { text } => code_span(text, mode, out),
        Inline::Link {
            target,
            title,
            wiki,
            inlines: content,
            ..
        } => {
            if link_depth > MAX_DEPTH {
                // Too deep to walk by recursion: the text stays, flattened.
                escape_with(&flat_text(content), line_mode(mode), dollars, out);
                return;
            }
            let merged = fix_flanking(merge_text(content));
            if link_depth > 0 {
                // A link inside a link text keeps only its text.
                for inner in &merged {
                    inline_into(inner, line_mode(mode), link_depth + 1, dollars, out);
                }
                return;
            }
            if *wiki && let Some(written) = wikilink(target, content) {
                out.push_str(&written);
                return;
            }
            if !safe_target(target) {
                // Never a live link: the text, then the target as code.
                for inner in &merged {
                    inline_into(inner, line_mode(mode), 1, dollars, out);
                }
                out.push(' ');
                code_span(target, line_mode(mode), out);
                return;
            }
            out.push('[');
            for inner in &merged {
                inline_into(inner, line_mode(mode), 1, dollars, out);
            }
            out.push_str("](");
            destination(target, title.as_deref(), mode, out);
            out.push(')');
        }
        Inline::FootnoteRef { id } => {
            out.push_str("[^");
            out.push_str(&footnote_label(id, mode));
            out.push(']');
        }
        Inline::Math { latex } => inline_math(latex, mode, out),
        Inline::Status { text, color } => {
            out.push_str(":status[");
            escape_with(text, line_mode(mode), dollars, out);
            out.push_str("]{");
            out.push_str(status_color(*color));
            out.push('}');
        }
        // Anchors are kept in the model JSON, not in the Markdown.
        Inline::AnchorMark { .. } => {}
    }
}

fn line_mode(mode: Mode) -> Mode {
    if mode == Mode::Block {
        Mode::Line
    } else {
        mode
    }
}

fn styled_text(
    text: &str,
    emphasis: bool,
    strong: bool,
    strike: bool,
    mode: Mode,
    dollars: bool,
    out: &mut String,
) {
    if !(emphasis || strong || strike) {
        escape_with(text, mode, dollars, out);
        return;
    }
    // Delimiters must touch the text, so outer whitespace goes outside them.
    let core = text.trim();
    if core.is_empty() {
        escape_with(text, mode, dollars, out);
        return;
    }
    let start = text.len() - text.trim_start().len();
    let end = start + core.len();
    escape_with(&text[..start], mode, dollars, out);
    let mut open = String::new();
    if strike {
        open.push_str("~~");
    }
    if strong {
        open.push_str("**");
    }
    if emphasis {
        open.push('*');
    }
    let close: String = open.chars().rev().collect();
    out.push_str(&open);
    escape_with(core, mode, dollars, out);
    out.push_str(&close);
    escape_with(&text[end..], mode, dollars, out);
}

/// Escapes text from a file so that it stays text.
///
/// Line-start markers (`#`, `>`, `-`, `+`, `=`, `1.`) are handled per line in
/// [`finish_lines`]; everything that is structure anywhere in a line is handled here.
pub(crate) fn escape_text(text: &str, mode: Mode, out: &mut String) {
    escape_with(text, mode, false, out);
}

/// [`escape_text`]; with `dollars` every `$` is escaped (the block has a pair of them
/// across its runs).
fn escape_with(text: &str, mode: Mode, dollars: bool, out: &mut String) {
    let chars: Vec<char> = text.chars().collect();
    let mut prev: Option<char> = None;
    // A `$` with another `$` later in the run could pair into math in GitHub's reading.
    let last_dollar = chars.iter().rposition(|c| *c == '$');
    for (i, &c) in chars.iter().enumerate() {
        let next = chars.get(i + 1).copied();
        match c {
            '\\' | '`' | '*' | '_' | '[' | ']' | '~' | '|' | '&' => {
                out.push('\\');
                out.push(c);
            }
            '<' => out.push_str("&lt;"),
            // `$` before a backtick or another `$` opens math in Crystalline, a pair of single
            // `$` does in GitHub; at the end of a run the next character is unknown (a code
            // span may follow).
            '$' if matches!(next, Some('`') | Some('$') | None)
                || prev == Some('$')
                || dollars
                || last_dollar.is_some_and(|l| i < l) =>
            {
                out.push_str("\\$");
            }
            '\r' if next == Some('\n') => {}
            '\n' | '\r' => {
                if mode == Mode::Block {
                    out.push('\n');
                } else {
                    out.push(' ');
                }
            }
            _ => out.push(c),
        }
        prev = Some(c);
    }
}

/// Escapes plain text for one line (a title, a cell).
pub(crate) fn line_text(text: &str) -> String {
    let mut out = String::new();
    escape_text(text, Mode::Line, &mut out);
    out
}

/// Escapes plain text for a table cell.
pub(crate) fn cell_text(text: &str) -> String {
    let mut out = String::new();
    escape_text(text, Mode::Cell, &mut out);
    out
}

/// The longest run of `ch` in `text`.
fn longest_run(text: &str, ch: char) -> usize {
    let mut best = 0;
    let mut run = 0;
    for c in text.chars() {
        if c == ch {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}

fn one_line(text: &str, mode: Mode) -> String {
    let mut s: String = text
        .chars()
        .map(|c| if c.is_control() && c != '\t' { ' ' } else { c })
        .collect();
    if mode == Mode::Cell {
        s = s.replace('|', "\\|");
    }
    s
}

/// Inline code in a span delimited by more backticks than any run inside.
pub(crate) fn code_span(text: &str, mode: Mode, out: &mut String) {
    if text.is_empty() {
        return;
    }
    let content = one_line(text, mode);
    let delim = "`".repeat(longest_run(&content, '`') + 1);
    let pad = content.starts_with('`')
        || content.ends_with('`')
        || (content.starts_with(' ') && content.ends_with(' ') && !content.trim().is_empty());
    out.push_str(&delim);
    if pad {
        out.push(' ');
    }
    out.push_str(&content);
    if pad {
        out.push(' ');
    }
    out.push_str(&delim);
}

fn inline_math(latex: &str, mode: Mode, out: &mut String) {
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

/// A link or image destination plus optional title, without the parentheses.
/// The plain text of inlines, walked with a stack instead of recursion.
fn flat_text(list: &[Inline]) -> String {
    let mut out = String::new();
    let mut stack: Vec<std::slice::Iter<'_, Inline>> = vec![list.iter()];
    while let Some(top) = stack.last_mut() {
        let Some(inline) = top.next() else {
            stack.pop();
            continue;
        };
        match inline {
            Inline::Text { text, .. } | Inline::Code { text } => out.push_str(text),
            Inline::Link { inlines, .. } => stack.push(inlines.iter()),
            Inline::Math { latex } => out.push_str(latex),
            Inline::Image { alt, .. } => out.push_str(alt),
            Inline::Status { text, .. } => out.push_str(text),
            Inline::LineBreak => out.push(' '),
            Inline::FootnoteRef { .. } | Inline::AnchorMark { .. } => {}
        }
    }
    out
}

/// True for a target with no scheme (a relative path or `#anchor`) or with http, https,
/// mailto, ftp or file. Tabs and newlines are ignored and leading spaces and controls
/// trimmed first, as browsers do.
fn safe_target(target: &str) -> bool {
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
    matches!(
        scheme.to_ascii_lowercase().as_str(),
        "http" | "https" | "mailto" | "ftp" | "file"
    )
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

fn destination(target: &str, title: Option<&str>, mode: Mode, out: &mut String) {
    if target.is_empty() {
        out.push_str("<>");
    }
    for c in target.chars() {
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
fn footnote_label(id: &str, mode: Mode) -> String {
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

pub(crate) fn status_color(color: StatusColor) -> &'static str {
    match color {
        StatusColor::Grey => "grey",
        StatusColor::Blue => "blue",
        StatusColor::Green => "green",
        StatusColor::Yellow => "yellow",
        StatusColor::Red => "red",
        StatusColor::Purple => "purple",
    }
}

/// Makes a rendered paragraph safe line by line: blank lines go (they would end the
/// paragraph), the first line loses its indent (it would become code) and every line that
/// starts with a block marker gets it escaped.
pub(crate) fn finish_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut first = true;
    for line in text.split('\n') {
        if line.trim().is_empty() {
            continue;
        }
        if !first {
            out.push('\n');
        }
        let line = if first { line.trim_start() } else { line };
        first = false;
        let indent = line.len() - line.trim_start().len();
        out.push_str(&line[..indent]);
        escape_line_start(&line[indent..], &mut out);
    }
    out
}

fn escape_line_start(line: &str, out: &mut String) {
    match line.chars().next() {
        Some('#' | '>' | '-' | '+' | '=') => {
            out.push('\\');
            out.push_str(line);
        }
        Some(c) if c.is_ascii_digit() => {
            let digits = line.chars().take_while(char::is_ascii_digit).count();
            match line[digits..].chars().next() {
                Some('.' | ')') => {
                    out.push_str(&line[..digits]);
                    out.push('\\');
                    out.push_str(&line[digits..]);
                }
                _ => out.push_str(line),
            }
        }
        _ => out.push_str(line),
    }
}

/// A paragraph from inlines; empty when there is no visible content.
pub(crate) fn paragraph(list: &[Inline]) -> String {
    let start = list
        .iter()
        .position(|i| !matches!(i, Inline::LineBreak))
        .unwrap_or(list.len());
    let end = list
        .iter()
        .rposition(|i| !matches!(i, Inline::LineBreak))
        .map_or(start, |e| e + 1);
    finish_lines(&inlines(&list[start..end.max(start)], Mode::Block))
}

// ---------------------------------------------------------------------------------------
// Blocks

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrevKind {
    None,
    Bullet,
    Ordered,
    Other,
}

#[derive(Debug, Clone, Copy)]
struct Prev {
    kind: PrevKind,
    alt: bool,
    task: bool,
    /// A paragraph, list or task: a following list may join it tightly.
    tight_lead: bool,
}

/// Renders a block sequence. `level` is the heading level for titled sections; `tight`
/// joins a paragraph and a following list without a blank line (inside list items).
pub(crate) fn blocks(list: &[Block], ctx: &Ctx, depth: usize, level: u8, tight: bool) -> String {
    if depth > MAX_DEPTH {
        return TOO_DEEP.into();
    }
    let mut out = String::new();
    let mut prev = Prev {
        kind: PrevKind::None,
        alt: false,
        task: false,
        tight_lead: false,
    };
    for item in list {
        let (text, now) = match item {
            Block::Task { state, inlines, .. } => {
                let alt = if prev.task {
                    prev.alt
                } else {
                    prev.kind == PrevKind::Bullet && !prev.alt
                };
                (
                    task(*state, inlines, alt),
                    Prev {
                        kind: PrevKind::Bullet,
                        alt,
                        task: true,
                        tight_lead: true,
                    },
                )
            }
            Block::List { ordered, items, .. } => {
                let same = if *ordered {
                    PrevKind::Ordered
                } else {
                    PrevKind::Bullet
                };
                let alt = prev.kind == same && !prev.alt;
                (
                    list_block(*ordered, items, alt, ctx, depth, level),
                    Prev {
                        kind: same,
                        alt,
                        task: false,
                        tight_lead: true,
                    },
                )
            }
            other => (
                block(other, ctx, depth, level),
                Prev {
                    kind: PrevKind::Other,
                    alt: false,
                    task: false,
                    tight_lead: matches!(other, Block::Paragraph { .. }),
                },
            ),
        };
        if text.is_empty() {
            continue;
        }
        if !out.is_empty() {
            let joins_tight = (prev.task && now.task)
                || (tight && prev.tight_lead && now.kind != PrevKind::Other);
            out.push_str(if joins_tight { "\n" } else { "\n\n" });
        }
        out.push_str(&text);
        prev = now;
    }
    out
}

fn block(b: &Block, ctx: &Ctx, depth: usize, level: u8) -> String {
    match b {
        Block::Section {
            title,
            anchor,
            blocks: inner,
            ..
        } => {
            let label = anchor.as_ref().and_then(|a| a.0.strip_prefix("fn:"));
            match (title.as_deref().filter(|t| !t.trim().is_empty()), label) {
                (None, Some(label)) => footnote(label, inner, ctx, depth),
                (Some(title), _) => {
                    let head = heading_line(level.min(6), &line_text(title.trim()));
                    let body = blocks(inner, ctx, depth + 1, level.saturating_add(1), false);
                    join2(head, body)
                }
                (None, None) => blocks(inner, ctx, depth + 1, level, false),
            }
        }
        Block::Heading {
            level: l,
            inlines: content,
            ..
        } => {
            let l = l.saturating_add(ctx.heading_offset).clamp(1, 6);
            heading_line(l, inlines(content, Mode::Line).trim())
        }
        Block::Paragraph {
            inlines: content, ..
        } => paragraph(content),
        Block::List { ordered, items, .. } => list_block(*ordered, items, false, ctx, depth, level),
        Block::Table {
            caption,
            header_rows,
            rows,
            alignments,
            ..
        } => model_table(caption.as_deref(), *header_rows, rows, alignments),
        Block::Code { lang, text, .. } => fence(lang.as_deref(), text),
        Block::Quote { blocks: inner, .. } => {
            let body = blocks(inner, ctx, depth + 1, level, false);
            prefix_lines(&body, ">")
        }
        Block::Math { latex, .. } => math_block(latex),
        Block::Media { asset, .. } => media(asset, ctx),
        Block::Callout {
            kind,
            title,
            folded,
            blocks: inner,
            ..
        } => {
            let mut head = format!("[!{}]", callout_kind(*kind));
            match folded {
                Some(true) => head.push('-'),
                Some(false) => head.push('+'),
                None => {}
            }
            if let Some(title) = title.as_deref().filter(|t| !t.trim().is_empty()) {
                head.push(' ');
                head.push_str(&line_text(title.trim()));
            }
            let body = blocks(inner, ctx, depth + 1, level, false);
            prefix_lines(&join_lines(head, body), ">")
        }
        Block::Rule { .. } => "---".into(),
        Block::Task { state, inlines, .. } => task(*state, inlines, false),
    }
}

fn join2(a: String, b: String) -> String {
    if b.is_empty() {
        a
    } else {
        format!("{a}\n\n{b}")
    }
}

fn join_lines(a: String, b: String) -> String {
    if b.is_empty() { a } else { format!("{a}\n{b}") }
}

/// `#` times `level`, then the text with a trailing `#` run escaped (it would close the
/// heading).
pub(crate) fn heading_line(level: u8, text: &str) -> String {
    let hashes = "#".repeat(usize::from(level.max(1)));
    if text.is_empty() {
        return hashes;
    }
    let trimmed = text.trim_end_matches('#');
    if trimmed.len() == text.len() {
        format!("{hashes} {text}")
    } else {
        format!("{hashes} {trimmed}\\{}", &text[trimmed.len()..])
    }
}

fn callout_kind(kind: CalloutKind) -> &'static str {
    match kind {
        CalloutKind::Note => "NOTE",
        CalloutKind::Tip => "TIP",
        CalloutKind::Important => "IMPORTANT",
        CalloutKind::Warning => "WARNING",
        CalloutKind::Caution => "CAUTION",
        CalloutKind::Details => "DETAILS",
    }
}

/// Prefixes every line with `marker` (`>`), with a space before non-empty lines.
fn prefix_lines(text: &str, marker: &str) -> String {
    if text.is_empty() {
        return marker.into();
    }
    text.split('\n')
        .map(|l| {
            if l.is_empty() {
                marker.to_string()
            } else {
                format!("{marker} {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The first line after `first`, the rest indented by `indent` spaces.
fn hang(first: &str, body: &str, indent: usize) -> String {
    if body.is_empty() {
        return first.trim_end().to_string();
    }
    let pad = " ".repeat(indent);
    let mut out = String::new();
    for (i, line) in body.split('\n').enumerate() {
        if i == 0 {
            out.push_str(first);
            out.push_str(line);
        } else {
            out.push('\n');
            if !line.is_empty() {
                out.push_str(&pad);
                out.push_str(line);
            }
        }
    }
    out
}

fn list_block(
    ordered: bool,
    items: &[Vec<Block>],
    alt: bool,
    ctx: &Ctx,
    depth: usize,
    level: u8,
) -> String {
    if depth > MAX_DEPTH {
        return TOO_DEEP.into();
    }
    // Loose when an item has blocks that need a blank line between them; then every item
    // keeps blank lines, also before a nested list.
    let loose = items.iter().any(|item| {
        item.windows(2).any(|w| {
            let lead = matches!(
                w[0],
                Block::Paragraph { .. } | Block::List { .. } | Block::Task { .. }
            );
            let listy = matches!(w[1], Block::List { .. } | Block::Task { .. });
            !(lead && listy)
        })
    });
    let rendered: Vec<String> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let marker = if ordered {
                format!("{}{} ", i + 1, if alt { ')' } else { '.' })
            } else if alt {
                "* ".to_string()
            } else {
                "- ".to_string()
            };
            let body = blocks(item, ctx, depth + 1, level, !loose);
            hang(&marker, &body, marker.len())
        })
        .collect();
    rendered.join(if loose { "\n\n" } else { "\n" })
}

fn task(state: TaskState, content: &[Inline], alt: bool) -> String {
    let mark = match state {
        TaskState::Open => ' ',
        TaskState::Done => 'x',
        TaskState::Cancelled => '~',
    };
    let bullet = if alt { '*' } else { '-' };
    let first = format!("{bullet} [{mark}] ");
    hang(&first, &paragraph(content), 2)
}

fn footnote(label: &str, inner: &[Block], ctx: &Ctx, depth: usize) -> String {
    let body = blocks(inner, ctx, depth + 1, 6, false);
    hang(
        &format!("[^{}]: ", footnote_label(label, Mode::Block)),
        &body,
        4,
    )
}

/// A fenced block with a fence longer than any backtick run inside.
pub(crate) fn fence(lang: Option<&str>, text: &str) -> String {
    let fence = "`".repeat(longest_run(text, '`').max(2) + 1);
    let info: String = lang
        .unwrap_or("")
        .chars()
        .take_while(|c| !c.is_whitespace())
        .filter(|c| *c != '`')
        .collect();
    let body = text.strip_suffix('\n').unwrap_or(text);
    if body.is_empty() {
        format!("{fence}{info}\n{fence}")
    } else {
        format!("{fence}{info}\n{body}\n{fence}")
    }
}

fn math_block(latex: &str) -> String {
    let body = latex.trim_matches('\n');
    if body.trim().is_empty() {
        return String::new();
    }
    if math_is_plain(body) {
        format!("$$\n{body}\n$$")
    } else {
        fence(Some("math"), body)
    }
}

/// True when `$$ ... $$` is safe: no blank line, no `$$`, no `<` or backtick, and no line
/// that a plain Markdown reader would take as a block marker.
fn math_is_plain(body: &str) -> bool {
    if body.contains("$$") || body.contains('<') || body.contains('`') {
        return false;
    }
    body.split('\n').all(|line| {
        let t = line.trim_start();
        let Some(c) = t.chars().next() else {
            return false;
        };
        if c.is_ascii_digit() {
            let digits = t.chars().take_while(char::is_ascii_digit).count();
            return !matches!(t[digits..].chars().next(), Some('.' | ')'));
        }
        c.is_alphanumeric()
            || matches!(
                c,
                '\\' | '{'
                    | '}'
                    | '('
                    | ')'
                    | '^'
                    | '_'
                    | '&'
                    | ','
                    | '.'
                    | ';'
                    | ':'
                    | '\''
                    | '!'
                    | '?'
                    | '%'
                    | '/'
                    | '"'
            )
    })
}

fn media(id: &str, ctx: &Ctx) -> String {
    let mut parts: Vec<Inline> = Vec::new();
    let text = |t: String| Inline::Text {
        text: t,
        emphasis: false,
        strong: false,
        strike: false,
    };
    match ctx.assets.iter().find(|a| a.id == id) {
        None => parts.push(text(format!("Picture {id} is missing from the assets."))),
        Some(asset) => {
            match &asset.path {
                Some(path) => parts.push(Inline::Image {
                    target: path.clone(),
                    alt: asset
                        .alt
                        .clone()
                        .or_else(|| asset.caption.clone())
                        .unwrap_or_else(|| asset.id.clone()),
                    title: None,
                }),
                None => parts.push(text(format!(
                    "Picture {} ({}) is not extracted.",
                    asset.id, asset.mime
                ))),
            }
            if let Some(caption) = asset.caption.as_ref().filter(|c| !c.trim().is_empty()) {
                parts.push(text(" ".into()));
                parts.push(Inline::Text {
                    text: caption.clone(),
                    emphasis: true,
                    strong: false,
                    strike: false,
                });
            }
            if asset.needs_vision {
                parts.push(text(" ".into()));
                parts.push(Inline::Status {
                    text: "Needs vision".into(),
                    color: StatusColor::Yellow,
                });
            }
        }
    }
    paragraph(&parts)
}

// ---------------------------------------------------------------------------------------
// Tables

/// A GFM table from cells that are already escaped single-line Markdown.
pub(crate) fn pipe_table(header: &[String], aligns: &[Align], rows: &[Vec<String>]) -> String {
    let width = rows
        .iter()
        .map(Vec::len)
        .chain(std::iter::once(header.len()))
        .max()
        .unwrap_or(0)
        .max(1);
    let line = |cells: &[String]| {
        let mut s = String::from("|");
        for i in 0..width {
            s.push(' ');
            s.push_str(cells.get(i).map_or("", String::as_str));
            s.push_str(" |");
        }
        s
    };
    let mut out = line(header);
    out.push_str("\n|");
    for i in 0..width {
        out.push_str(match aligns.get(i).copied().unwrap_or(Align::None) {
            Align::None => " --- |",
            Align::Left => " :--- |",
            Align::Center => " :---: |",
            Align::Right => " ---: |",
        });
    }
    for row in rows {
        out.push('\n');
        out.push_str(&line(row));
    }
    out
}

/// A table shows at most this many columns.
const MAX_COLUMNS: usize = 1_000;

/// A table's rendered grid (rows times width) holds at most this many cells.
const MAX_CELLS: usize = 200_000;

fn model_table(
    caption: Option<&str>,
    header_rows: usize,
    rows: &[Vec<Cell>],
    aligns: &[Align],
) -> String {
    if rows.is_empty() {
        return caption
            .filter(|c| !c.trim().is_empty())
            .map(|c| format!("**{}** (empty table)", line_text(c.trim())))
            .unwrap_or_default();
    }
    // The real column count: per row the sum of its spans where that is plausible, else
    // its number of cells. Spans never reach past it.
    let real = rows
        .iter()
        .map(|row| {
            let sum = row
                .iter()
                .map(|c| c.col_span.max(1) as usize)
                .fold(0usize, usize::saturating_add);
            if sum <= MAX_COLUMNS { sum } else { row.len() }
        })
        .max()
        .unwrap_or(1)
        .clamp(1, MAX_COLUMNS);
    // Place every cell on a grid; spanned positions repeat the text. Rows stop once the
    // grid would pass MAX_CELLS; the header row is always shown.
    let mut grid: Vec<Vec<Option<String>>> = vec![Vec::new(); rows.len()];
    // (row, column, column span, row span) of every merged cell.
    let mut merged: Vec<(usize, usize, usize, usize)> = Vec::new();
    let mut filled = 0usize;
    let mut width = 0usize;
    let mut wanted = 0usize;
    let mut shown = rows.len();
    for (r, row) in rows.iter().enumerate() {
        let mut c = 0;
        for cell in row {
            while grid[r].get(c).is_some_and(Option::is_some) {
                c += 1;
            }
            if c >= MAX_COLUMNS {
                wanted = wanted.max(c + 1);
                c += 1;
                continue;
            }
            let text = inlines(&cell.inlines, Mode::Cell).trim().to_string();
            let rs = (cell.row_span.max(1) as usize).min(rows.len() - r);
            let cs = (cell.col_span.max(1) as usize)
                .min(real.saturating_sub(c).max(1))
                .min(MAX_COLUMNS - c);
            // Past the cell budget a span fills only its own cell; the note still names it.
            let (fill_rows, fill_cols) = if filled.saturating_add(rs * cs) > MAX_CELLS {
                (1, 1)
            } else {
                (rs, cs)
            };
            filled += fill_rows * fill_cols;
            for grid_row in &mut grid[r..r + fill_rows] {
                if grid_row.len() < c + fill_cols {
                    grid_row.resize(c + fill_cols, None);
                }
                for slot in &mut grid_row[c..c + fill_cols] {
                    *slot = Some(text.clone());
                }
            }
            if rs > 1 || cs > 1 {
                merged.push((r, c, cs, rs));
            }
            c += cs;
            wanted = wanted.max(c);
        }
        width = width.max(grid[r].len());
        if r > 0 && (r + 1).saturating_mul(width.max(1)) > MAX_CELLS {
            shown = r;
            break;
        }
    }
    grid.truncate(shown);
    let grid: Vec<Vec<String>> = grid
        .into_iter()
        .map(|row| row.into_iter().map(Option::unwrap_or_default).collect())
        .collect();
    let (header, body) = if header_rows == 0 {
        (Vec::new(), &grid[..])
    } else {
        (grid[0].clone(), &grid[1..])
    };
    let mut out = String::new();
    if let Some(caption) = caption.filter(|c| !c.trim().is_empty()) {
        out.push_str(&format!("**{}**\n\n", line_text(caption.trim())));
    }
    out.push_str(&pipe_table(&header, aligns, body));
    let mut notes: Vec<String> = Vec::new();
    // Only rows that are shown are named; a span into hidden rows ends at the last one.
    let merged: Vec<String> = merged
        .into_iter()
        .filter(|(r, ..)| *r < shown)
        .map(|(r, c, cs, rs)| {
            format!(
                "{}{}:{}{}",
                column_letter(c),
                r + 1,
                column_letter(c + cs - 1),
                (r + rs).min(shown)
            )
        })
        .collect();
    if !merged.is_empty() {
        notes.push(format!("Merged cells: {}", merged.join(", ")));
    }
    if shown < rows.len() {
        notes.push(format!(
            "{} more rows not shown (see the model JSON).",
            rows.len() - shown
        ));
    }
    if wanted > MAX_COLUMNS {
        notes.push(format!(
            "{} more columns not shown (see the model JSON).",
            wanted - MAX_COLUMNS
        ));
    }
    if !notes.is_empty() {
        out.push_str("\n\n");
        out.push_str(&notes.join("\n"));
    }
    out
}

/// `A`, `B`, ..., `Z`, `AA`, ... for a 0-based column.
fn column_letter(mut index: usize) -> String {
    let mut letters = Vec::new();
    loop {
        letters.push(char::from(b'A' + (index % 26) as u8));
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    letters.iter().rev().collect()
}

/// Characters allowed in a wikilink written as `[[...]]`: letters and digits of any script,
/// space and `- _ . / # ' ,`. Nothing here has a Markdown or HTML meaning inside `[[ ]]`.
fn wiki_plain(text: &str) -> bool {
    text.chars()
        .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '/' | '#' | '\'' | ','))
}

/// `[[target]]` or `[[target|text]]`, for a link that came from a wikilink. None unless the
/// target (without a `:`) and the text are plain (see [`wiki_plain`]) and the text has no
/// formatting, image or code; the link is then written as a normal, escaped link.
fn wikilink(target: &str, content: &[Inline]) -> Option<String> {
    let mut text = String::new();
    for inline in content {
        match inline {
            Inline::Text {
                text: t,
                emphasis: false,
                strong: false,
                strike: false,
            } => text.push_str(t),
            Inline::AnchorMark { .. } => {}
            _ => return None,
        }
    }
    if target.is_empty() || !wiki_plain(target) || !wiki_plain(&text) {
        return None;
    }
    if text.is_empty() || text == target {
        Some(format!("[[{target}]]"))
    } else {
        Some(format!("[[{target}|{text}]]"))
    }
}
