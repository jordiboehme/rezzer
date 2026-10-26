// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! List items that Crystalline would read as its own syntax (ruling P5-S).
//!
//! Crystalline 0.29.0 reads every bullet that starts a line with `- ` (no indent) as an
//! observation when its text opens with `[category]`, and as a relation when one token or a
//! quoted phrase stands before a `[[target]]` or before a link to a relative `.md` file that
//! opens at the first `[`. It reads the bullet over its first line and every line that
//! continues its first paragraph. Rezzer writes such an item from a file with the `* `
//! bullet, which Crystalline reads as plain text, so only the relations Rezzer means are
//! relations. The rules here are facts about that reading, not Crystalline's code.

use std::borrow::Cow;

/// `md` with the marker of every item Crystalline would read as an observation or a
/// relation changed from `- ` to `* `. Unchanged (borrowed) when there is none.
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
    let mut out = String::with_capacity(md.len());
    let mut next = flagged.into_iter().peekable();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if next.peek() == Some(&i) {
            next.next();
            out.push_str("* ");
            out.push_str(&line[2..]);
        } else {
            out.push_str(line);
        }
    }
    Cow::Owned(out)
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

/// Whether the last `[[` of a line has no `]]` after it (inline code included: the guard
/// only errs towards joining less, which reads at most what Crystalline reads per line).
fn ends_inside_wikilink(line: &str) -> bool {
    line.rfind("[[")
        .is_some_and(|open| !line[open + 2..].contains("]]"))
}

/// The type before a relation's target: a quoted phrase, or one token that does not open
/// with `:` or `[`.
fn relation_type(before: &str) -> bool {
    let before = before.trim();
    (before.len() >= 2 && before.starts_with('"') && before.ends_with('"'))
        || (!before.is_empty()
            && !before.contains(char::is_whitespace)
            && !before.starts_with([':', '[']))
}

/// Whether Crystalline reads a bullet's text as an observation or a relation. Links in
/// inline code count too: the guard may change a marker more than needed, never less.
fn read_by_crystalline(content: &str) -> bool {
    // An observation: `[category]` not followed by `(` or `[`, not a task box.
    if let Some(rest) = content.strip_prefix('[')
        && let Some(close) = rest.find(']')
    {
        let category = &rest[..close];
        let after = &rest[close + 1..];
        if !category.is_empty()
            && !category.contains('[')
            && !matches!(category, " " | "x" | "X" | "~")
            && !after.starts_with(['(', '['])
        {
            return true;
        }
    }
    // `type [[target]]`.
    if let Some(open) = content.find("[[")
        && content[open + 2..].contains("]]")
        && relation_type(&content[..open])
    {
        return true;
    }
    // `type [text](path.md)`, the link opening at the first `[`.
    let Some(open) = content.find('[') else {
        return false;
    };
    if !relation_type(&content[..open]) {
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
/// escapes and percent codes. Read loosely, so more paths look like notes, never fewer.
fn destination(line: &str, open: usize) -> Option<String> {
    let rest = &line[open..];
    let target = match rest.strip_prefix('<') {
        Some(inner) => &inner[..inner.find('>')?],
        None => {
            let end = rest
                .find(|c: char| c.is_whitespace() || c == ')')
                .unwrap_or(rest.len());
            &rest[..end]
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
    use super::guard;

    #[test]
    fn items_crystalline_reads_get_a_star() {
        assert_eq!(
            guard("- See [Three](chapter-3.md#c)"),
            "* See [Three](chapter-3.md#c)"
        );
        assert_eq!(guard("- [^1] text"), "* [^1] text");
        assert_eq!(guard("- tok [[x]]"), "* tok [[x]]");
        assert_eq!(
            guard("- \"two words\" [x](y.md)"),
            "* \"two words\" [x](y.md)"
        );
        // Over the lines that continue the item, as a hard break writes them.
        assert_eq!(
            guard("a\n\n- See\\\n  [x](y.md)\n- b"),
            "a\n\n* See\\\n  [x](y.md)\n- b"
        );
        // Not read: more words before the link, a web page, a task box, an indent, a fence,
        // a link that opens later, an escaped bracket.
        for md in [
            "- see this [x](y.md)",
            "- See [x](https://e.org/y.md)",
            "- [ ] task [x](y.md)",
            "  - See [x](y.md)",
            "```\n- See [x](y.md)\n```",
            "- See [x](a.png) and [y](z.md)",
            "- See \\[x\\](y.md)",
            "- [Text](x.md): a link first",
            "- index [x](index.md)",
        ] {
            assert_eq!(guard(md), md, "{md}");
        }
    }
}
