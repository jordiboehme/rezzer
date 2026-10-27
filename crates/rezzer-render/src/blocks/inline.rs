// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Inlines as Markdown: escaping, flanking runs of emphasis and text from lines and cells.

use super::*;

/// Renders inlines. In [`Mode::Block`] the result still needs [`finish_lines`].
pub(crate) fn inlines(list: &[Inline], mode: Mode) -> String {
    // Merging and the flanking fix only change styled runs and neighbouring text runs of the
    // same style; without those the list is used as it is, so a huge run is not copied.
    let prepared: Cow<'_, [Inline]> = if needs_preparing(list) {
        Cow::Owned(fix_flanking(merge_text(list)))
    } else {
        Cow::Borrowed(list)
    };
    let merged: &[Inline] = &prepared;
    let dollars = count_dollars(merged) >= 2;
    let mut out = String::new();
    for inline in merged {
        inline_into(inline, mode, 0, dollars, &mut out);
    }
    out
}

/// Whether [`merge_text`] or [`fix_flanking`] would change `list`: it has a styled text run
/// or two neighbouring text runs.
fn needs_preparing(list: &[Inline]) -> bool {
    list.iter().any(is_styled)
        || list
            .windows(2)
            .any(|w| matches!(w, [Inline::Text { .. }, Inline::Text { .. }]))
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

pub(super) fn inline_into(
    inline: &Inline,
    mode: Mode,
    link_depth: usize,
    dollars: bool,
    out: &mut String,
) {
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
                escape_with(&plain_text(content), line_mode(mode), dollars, out);
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
            if *wiki {
                out.push_str(&wikilink(target, content, mode, dollars));
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

pub(super) fn line_mode(mode: Mode) -> Mode {
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
pub(super) fn escape_with(text: &str, mode: Mode, dollars: bool, out: &mut String) {
    out.reserve(text.len());
    let mut prev: Option<char> = None;
    // A `$` with another `$` later in the run could pair into math in GitHub's reading.
    let last_dollar = text.rfind('$');
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let next = chars.peek().map(|&(_, n)| n);
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
pub(super) fn longest_run(text: &str, ch: char) -> usize {
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

pub(super) fn one_line(text: &str, mode: Mode) -> String {
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
