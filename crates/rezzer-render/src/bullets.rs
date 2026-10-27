// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! List items that Crystalline would read as its own syntax (ruling P5-S).
//!
//! Crystalline (0.29.1 or later) reads a bullet that starts a line with `- ` (no indent) as
//! an observation when its text opens with `[category]` (a category that opens with `^` is
//! a footnote, not a category), and as a relation in two forms: any one token or a quoted
//! phrase before a `[[target]]`, or a quoted phrase or a lowercase token (`part_of`,
//! `über`) before a link to a relative `.md` file that opens at the first `[`. A `[[`
//! inside a link destination or inline code is no wikilink when the text holds a link. It
//! reads the bullet over its first line and every line that continues its first paragraph.
//! Rezzer writes such an item from a file with the `* ` bullet, which Crystalline reads as
//! plain text, so only the relations Rezzer means are relations. The rules here are facts
//! about that reading, not Crystalline's code.

use std::borrow::Cow;

/// `md` with the marker of every list that holds an item Crystalline would read as an
/// observation or a relation changed from `- ` to `* ` (`+ ` when a `* ` list stands right
/// next to it), for all items of that list. Unchanged (borrowed) when there is none.
pub(crate) fn guard(md: &str) -> Cow<'_, str> {
    if !md.starts_with("- ") && !md.contains("\n- ") {
        return Cow::Borrowed(md);
    }
    let lines: Vec<&str> = md.split('\n').collect();
    let fenced = in_fence(&lines);
    let mut flagged: Vec<usize> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if fenced[i] {
            continue;
        }
        let Some(first) = text(line).strip_prefix("- ") else {
            continue;
        };
        let mut content = first.trim_end().to_string();
        if !(first.starts_with('[') && !first.contains(']')) {
            let mut end = i;
            for (j, next) in lines.iter().enumerate().skip(i + 1) {
                if fenced[j] || !continues(text(next)) || ends_inside_wikilink(text(lines[end])) {
                    break;
                }
                content.push(' ');
                content.push_str(text(next).trim());
                end = j;
            }
        }
        if read_by_crystalline(&content) {
            flagged.push(i);
        }
    }
    if flagged.is_empty() {
        return Cow::Borrowed(md);
    }
    // Every item of a list with a flagged item gets the new marker, so the list stays one
    // list (P5-X). Nested lists are indented and keep their own markers.
    let mut marker: Vec<Option<&str>> = vec![None; lines.len()];
    let mut flagged = flagged.into_iter().peekable();
    while let Some(&at) = flagged.peek() {
        let (start, end, items) = list_around(&lines, &fenced, at);
        while flagged.peek().is_some_and(|f| *f < end) {
            flagged.next();
        }
        // A `* ` list right before or after would join this one: then `+ `.
        let star_next = |line: Option<&&str>| line.is_some_and(|l| text(l).starts_with("* "));
        let before = lines[..start]
            .iter()
            .rev()
            .find(|l| !text(l).trim().is_empty() && !text(l).starts_with([' ', '\t']));
        let after = lines[end..].iter().find(|l| !text(l).trim().is_empty());
        let new = if star_next(before) || star_next(after) {
            "+ "
        } else {
            "* "
        };
        for i in items {
            marker[i] = Some(new);
        }
    }
    let mut out = String::with_capacity(md.len());
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        match marker[i] {
            Some(new) => {
                out.push_str(new);
                out.push_str(&line[2..]);
            }
            None => out.push_str(line),
        }
    }
    Cow::Owned(out)
}

/// The list the zero-indent `- ` item at `at` belongs to: its first line, the line after its
/// end, and the lines of its items. A list goes on over blank lines, indented lines and
/// further `- ` items; any other line at the left edge ends it.
fn list_around(lines: &[&str], fenced: &[bool], at: usize) -> (usize, usize, Vec<usize>) {
    let item = |i: usize| !fenced[i] && text(lines[i]).starts_with("- ");
    let inside = |i: usize| {
        let t = text(lines[i]);
        t.trim().is_empty() || t.starts_with([' ', '\t']) || item(i)
    };
    let mut start = at;
    let mut i = at;
    while i > 0 && inside(i - 1) {
        i -= 1;
        if item(i) {
            start = i;
        }
    }
    let mut end = at + 1;
    while end < lines.len() && inside(end) {
        end += 1;
    }
    // Blank lines at the end belong after the list.
    while end > at + 1 && text(lines[end - 1]).trim().is_empty() {
        end -= 1;
    }
    let items = (start..end).filter(|&i| item(i)).collect();
    (start, end, items)
}

/// A line without its carriage return.
fn text(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

/// A fence marker: at most 3 spaces, then 3 or more of `` ` `` or `~`.
fn fence_marker(line: &str) -> Option<(char, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let first = rest.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let count = rest.chars().take_while(|c| *c == first).count();
    (count >= 3).then_some((first, count))
}

/// For each line, whether it is a fence line or inside a fence.
fn in_fence(lines: &[&str]) -> Vec<bool> {
    let mut open: Option<(char, usize)> = None;
    lines
        .iter()
        .map(|line| {
            let line = text(line);
            match open {
                None => {
                    open = fence_marker(line);
                    open.is_some()
                }
                Some((c, n)) => {
                    if let Some((c2, n2)) = fence_marker(line)
                        && c2 == c
                        && n2 >= n
                        && line.trim_start()[n2..].trim().is_empty()
                    {
                        open = None;
                    }
                    true
                }
            }
        })
        .collect()
}

fn is_heading(line: &str) -> bool {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = &line[indent..];
    let hashes = rest.chars().take_while(|c| *c == '#').count();
    indent <= 3
        && (1..=6).contains(&hashes)
        && rest[hashes..]
            .chars()
            .next()
            .is_none_or(|c| c == ' ' || c == '\t')
}

fn starts_list_item(t: &str) -> bool {
    let gap = |r: &str| r.is_empty() || r.starts_with([' ', '\t']);
    if let Some(r) = t.strip_prefix(['-', '*', '+']) {
        return gap(r);
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    (1..=9).contains(&digits)
        && t[..digits].parse::<u32>() == Ok(1)
        && t[digits..].strip_prefix(['.', ')']).is_some_and(gap)
}

fn is_thematic_break(t: &str) -> bool {
    let Some(first) = t.chars().next().filter(|c| matches!(c, '-' | '*' | '_')) else {
        return false;
    };
    t.chars().all(|c| c == first || c == ' ' || c == '\t')
        && t.chars().filter(|c| *c == first).count() >= 3
}

/// Whether a line after a bullet's text continues it.
fn continues(line: &str) -> bool {
    let t = line.trim();
    !(t.is_empty()
        || is_heading(line)
        || fence_marker(t).is_some()
        || starts_list_item(t)
        || t.starts_with(['>', '<', '|'])
        || is_thematic_break(t))
}

/// Whether the last `[[` of a line outside inline code and link destinations has no `]]`
/// after it. Crystalline masks both first; the guard must join exactly the lines
/// Crystalline joins, as a line joined less could hide a relation that Crystalline reads.
fn ends_inside_wikilink(line: &str) -> bool {
    if !line.contains("[[") {
        return false;
    }
    let code = mask_code(line);
    let masked = mask_destinations(&code);
    masked
        .rfind("[[")
        .is_some_and(|open| !masked[open + 2..].contains("]]"))
}

/// `line` with every inline code span, backticks included, replaced by spaces, byte for
/// byte, so offsets stay.
fn mask_code(line: &str) -> Cow<'_, str> {
    let spans = code_spans(line);
    if spans.is_empty() {
        return Cow::Borrowed(line);
    }
    let mut masked = line.as_bytes().to_vec();
    for (start, end) in spans {
        masked[start..end].fill(b' ');
    }
    // Whole spans are blanked, so the bytes are still UTF-8.
    Cow::Owned(String::from_utf8_lossy(&masked).into_owned())
}

/// `line` with the inside of every inline link destination, between `](` and its `)`,
/// replaced by spaces, byte for byte, so offsets stay. A `[[` in there is part of a path,
/// no wikilink. The destinations are found as Crystalline finds them: a `](` that closes a
/// link text and opens a destination that closes on the line. Call it on a line with
/// inline code masked, so a `](` in code opens nothing.
fn mask_destinations(line: &str) -> Cow<'_, str> {
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut cursor = 0;
    while let Some(found) = line[cursor..].find("](") {
        let close_text = cursor + found;
        let open = close_text + 2;
        cursor = open;
        if link_text_start(line, close_text).is_none() {
            continue;
        }
        let Some(close) = destination_end(line, open) else {
            continue;
        };
        ranges.push((open, close));
        cursor = close;
    }
    if ranges.is_empty() {
        return Cow::Borrowed(line);
    }
    let mut masked = line.as_bytes().to_vec();
    for (start, end) in ranges {
        masked[start..end].fill(b' ');
    }
    // A range runs from after `(` to before `)`, so it holds whole characters.
    Cow::Owned(String::from_utf8_lossy(&masked).into_owned())
}

/// The offset of the `)` that closes the destination after `](` at `open`: after the `>`
/// of the angle bracket form, else the closing parenthesis.
fn destination_end(line: &str, open: usize) -> Option<usize> {
    let rest = &line[open..];
    match rest.strip_prefix('<') {
        Some(inner) => {
            let after = open + 1 + inner.find('>')? + 1;
            Some(after + closing_paren(&line[after..])?)
        }
        None => Some(open + closing_paren(rest)?),
    }
}

/// The byte ranges of the inline code spans of a line, backticks included: a run of
/// backticks up to the next run of the same length.
fn code_spans(line: &str) -> Vec<(usize, usize)> {
    let b = line.as_bytes();
    let run_at = |i: usize| b[i..].iter().take_while(|c| **c == b'`').count();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'`' {
            i += 1;
            continue;
        }
        let run = run_at(i);
        let mut k = i + run;
        let mut closed = None;
        while k < b.len() {
            if b[k] == b'`' {
                let n = run_at(k);
                if n == run {
                    closed = Some(k + n);
                    break;
                }
                k += n;
            } else {
                k += 1;
            }
        }
        match closed {
            Some(end) => {
                spans.push((i, end));
                i = end;
            }
            None => i += run,
        }
    }
    spans
}

/// The type before a wikilink relation's target: a quoted phrase, or one token that does
/// not open with `:` or `[`.
fn relation_type(before: &str) -> bool {
    let before = before.trim();
    (before.len() >= 2 && before.starts_with('"') && before.ends_with('"'))
        || (!before.is_empty()
            && !before.contains(char::is_whitespace)
            && !before.starts_with([':', '[']))
}

/// The type before a relative-link relation's target: a quoted phrase, or a token whose
/// first character is a lowercase letter and whose others are lowercase letters, ASCII
/// digits, `_` or `-` (`part_of`, `über`). `See` in `- See [Three](chapter-3.md)` is prose.
fn relative_relation_type(before: &str) -> bool {
    let before = before.trim();
    if before.len() >= 2 && before.starts_with('"') && before.ends_with('"') {
        return true;
    }
    let mut chars = before.chars();
    chars.next().is_some_and(char::is_lowercase)
        && chars.all(|c| c.is_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Whether Crystalline reads a bullet's text as an observation or a relation. Links in
/// inline code count for the relative form: the guard may change a marker more than
/// needed, never less.
fn read_by_crystalline(content: &str) -> bool {
    // An observation: `[category]` not followed by `(` or `[`, not a task box, not a
    // footnote (`[^1]`).
    if let Some(rest) = content.strip_prefix('[')
        && let Some(close) = rest.find(']')
    {
        let category = &rest[..close];
        let after = &rest[close + 1..];
        if !category.is_empty()
            && !category.contains('[')
            && !category.starts_with('^')
            && !matches!(category, " " | "x" | "X" | "~")
            && !after.starts_with(['(', '['])
        {
            return true;
        }
    }
    // `type [[target]]`, the first `[[`. With a link in the text, a first `[[` in inline
    // code or in a link destination is part of a path or code, and no wikilink relation.
    if let Some(open) = content.find("[[")
        && content[open + 2..].contains("]]")
        && relation_type(&content[..open])
        && (!content.contains("](")
            || mask_destinations(&mask_code(content)).as_bytes()[open] == b'[')
    {
        return true;
    }
    // `type [text](path.md)`, the link opening at the first `[`.
    let Some(open) = content.find('[') else {
        return false;
    };
    if !relative_relation_type(&content[..open]) {
        return false;
    }
    let mut cursor = 0;
    while let Some(found) = content[cursor..].find("](") {
        let close_text = cursor + found;
        cursor = close_text + 2;
        if link_text_start(content, close_text) != Some(open) {
            continue;
        }
        if let Some(path) = destination(content, close_text + 2)
            && note_path(&path)
        {
            return true;
        }
    }
    false
}

/// The `[` that opens the link text closed at `close`: the nearest one at depth zero; a
/// bracket right after a backslash does not count.
fn link_text_start(line: &str, close: usize) -> Option<usize> {
    let b = line.as_bytes();
    let mut depth = 0usize;
    let mut i = close;
    while i > 0 {
        i -= 1;
        let escaped = i > 0 && b[i - 1] == b'\\';
        match b[i] {
            b']' if !escaped => depth += 1,
            b'[' if !escaped => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// The path of the destination after `](` at `open`: without its `#fragment`, backslash
/// escapes and percent codes. A bare one ends at an ASCII space or a control character, as
/// in Crystalline; U+3000 and U+00A0 are part of the path. Read loosely, so more paths look
/// like notes, never fewer.
fn destination(line: &str, open: usize) -> Option<String> {
    let rest = &line[open..];
    let target = match rest.strip_prefix('<') {
        Some(inner) => &inner[..inner.find('>')?],
        None => {
            let inside = &rest[..closing_paren(rest)?];
            let end = inside
                .find(|c: char| c == ' ' || c.is_ascii_control())
                .unwrap_or(inside.len());
            &inside[..end]
        }
    };
    let path = target.split('#').next().unwrap_or("");
    let path: String = path.chars().filter(|c| *c != '\\').collect();
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(v) = path
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

/// The offset of the `)` that closes a destination starting at `text`: balanced parentheses
/// inside are read past, and `\(`, `\)` and `\\` neither open nor close.
fn closing_paren(text: &str) -> Option<usize> {
    let b = text.as_bytes();
    let mut depth = 1usize;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' if matches!(b.get(i + 1), Some(b'(' | b')' | b'\\')) => i += 1,
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// A relative path to a `.md` file (any case), not a folder's `index.md` or `log.md`.
fn note_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with(['#', '/'])
        && !path.contains("://")
        && !path.starts_with("mailto:")
        && path.to_ascii_lowercase().ends_with(".md")
        && !matches!(path.rsplit('/').next(), Some("index.md" | "log.md"))
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::{ends_inside_wikilink, guard, mask_destinations};

    #[test]
    fn items_crystalline_reads_get_a_star() {
        // A relative-link relation needs a lowercase token or a quoted phrase as its type.
        assert_eq!(guard("- see [x](y.md)"), "* see [x](y.md)");
        assert_eq!(guard("- \u{fc}ber [x](y.md)"), "* \u{fc}ber [x](y.md)");
        assert_eq!(guard("- part_of [x](y.md)"), "* part_of [x](y.md)");
        assert_eq!(
            guard("- \"Two words\" [x](y.md)"),
            "* \"Two words\" [x](y.md)"
        );
        assert_eq!(
            guard("- \"two words\" [x](y.md)"),
            "* \"two words\" [x](y.md)"
        );
        // The wikilink form takes any one token.
        assert_eq!(guard("- Tok [[x]]"), "* Tok [[x]]");
        assert_eq!(guard("- tok [[x]]"), "* tok [[x]]");
        assert_eq!(guard("- [c] x"), "* [c] x");
        assert_eq!(guard("- [a^b] x"), "* [a^b] x");
        // Only an ASCII space or a control character ends a bare destination.
        assert_eq!(
            guard("- see [x](a\u{3000}b.md)"),
            "* see [x](a\u{3000}b.md)"
        );
        assert_eq!(guard("- see [x](a\u{a0}b.md)"), "* see [x](a\u{a0}b.md)");
        // Over the lines that continue the item, as a hard break writes them.
        assert_eq!(
            guard("a\n\n- Tok\\\n  [[x]]\n- b"),
            "a\n\n* Tok\\\n  [[x]]\n* b"
        );
        assert_eq!(guard("a\n\n- see\n  [x](y.md)"), "a\n\n* see\n  [x](y.md)");
        // The whole list changes, its nested lists keep their markers; a `* ` list next to
        // it makes the marker `+ `; a list after other text is a list of its own.
        assert_eq!(
            guard("- a\n  - n\n\n- [c] x\n- b\n\ntext\n\n- d"),
            "* a\n  - n\n\n* [c] x\n* b\n\ntext\n\n- d"
        );
        assert_eq!(guard("- [c] x\n- b\n\n* other"), "+ [c] x\n+ b\n\n* other");
        // A `[[` in inline code does not stop the join, as in Crystalline; then the joined
        // text holds a wikilink after one token.
        assert_eq!(guard("- tok`[[` then\n  x]]"), "* tok`[[` then\n  x]]");
        // Parentheses inside a destination are balanced, escaped ones skipped.
        assert_eq!(guard("- see [x](a_(b).md)"), "* see [x](a_(b).md)");
        assert_eq!(guard("- see [x](a\\).md)"), "* see [x](a\\).md)");
        // Not read: a type that is no lowercase token, more words before the link, a web
        // page, a task box, a footnote, an indent, a fence, a link that opens later, an
        // escaped bracket, a `[[` inside a link destination or inside inline code next to
        // one, a hard break in the type.
        for md in [
            "- See [Three](chapter-3.md#c)",
            "- Part_of [x](y.md)",
            "- partOf [x](y.md)",
            "- 2nd [x](y.md)",
            "- _x [x](y.md)",
            "- see: [x](y.md)",
            "- see this [x](y.md)",
            "- see [x](https://e.org/y.md)",
            "- [ ] task [x](y.md)",
            "- [^1] text",
            "- [^] text",
            "  - see [x](y.md)",
            "```\n- see [x](y.md)\n```",
            "- see [x](a.png) and [y](z.md)",
            "- see \\[x\\](y.md)",
            "- [Text](x.md): a link first",
            "- index [x](index.md)",
            "- Tok[x](a[[b]]c.png)",
            "- Tok`[x](a[[b]]c.png)`",
            "- see\\\n  [x](y.md)",
            "- See\\\n  [x](y.md)",
        ] {
            assert_eq!(guard(md), md, "{md}");
        }
    }

    #[test]
    fn destinations_are_masked_in_place() {
        assert_eq!(mask_destinations("a [b](c[[d) [[e]]"), "a [b](    ) [[e]]");
        assert_eq!(mask_destinations("[b](<c d[[>) x"), "[b](       ) x");
        // Multi-byte text keeps its byte length, so offsets stay.
        let line = "[b](\u{3000}[[x]].png) [[y]]";
        assert_eq!(mask_destinations(line).len(), line.len());
        assert!(matches!(
            mask_destinations("no link [[x]]"),
            Cow::Borrowed(_)
        ));
        // A `](` with no `[` before it opens no destination.
        assert_eq!(mask_destinations("a](b[[c]])"), "a](b[[c]])");
    }

    #[test]
    fn a_wikilink_opened_in_a_destination_or_code_does_not_hold_the_line() {
        assert!(ends_inside_wikilink("a [[b"));
        assert!(!ends_inside_wikilink("a [b](c[[d.png)"));
        assert!(!ends_inside_wikilink("a `[[` b"));
        assert!(!ends_inside_wikilink("a [[b]]"));
    }
}
