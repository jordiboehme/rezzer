// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Splits rendered Markdown into pages at section borders.
//!
//! Pages break before a `## ` line. A section larger than a page breaks before its `### `
//! lines, and a part still too large at a blank line. Breaks never fall inside a fence, the
//! frontmatter or the title, never before an indented line (it would turn into code) and
//! never before a `---` line (it would look like frontmatter), and a heading stays with the
//! block after it. Joining the pages gives the input back unchanged.

/// The token estimate: characters divided by four.
pub(crate) fn tokens(text: &str) -> usize {
    text.chars().count() / 4
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Border {
    None,
    H2,
    H3,
    Blank,
}

/// A fence opener or closer: up to three spaces, then three or more backticks or tildes.
fn fence_run(line: &str) -> Option<(char, usize, &str)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let ch = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let run = rest.chars().take_while(|c| *c == ch).count();
    (run >= 3).then(|| (ch, run, &rest[run..]))
}

/// The border before each line, and each line's start offset.
fn borders(md: &str) -> (Vec<usize>, Vec<Border>) {
    let lines: Vec<&str> = md.split_inclusive('\n').collect();
    let mut starts = Vec::with_capacity(lines.len());
    let mut kinds = Vec::with_capacity(lines.len());
    let mut offset = 0;

    // Frontmatter and the title heading right after it are never split.
    let mut protected_until = 0;
    if lines.first().map(|l| l.trim_end()) == Some("---")
        && let Some(end) = lines.iter().skip(1).position(|l| l.trim_end() == "---")
    {
        protected_until = end + 2;
    }
    let mut i = protected_until;
    while i < lines.len() && lines[i].trim().is_empty() {
        i += 1;
    }
    if i < lines.len() && lines[i].starts_with("# ") {
        protected_until = i + 1;
    }

    let mut fence: Option<(char, usize)> = None;
    let mut prev_blank = false;
    // A heading stays with the block after it.
    let mut after_heading = false;
    for (n, line) in lines.iter().enumerate() {
        starts.push(offset);
        offset += line.len();
        let text = line.trim_end_matches(['\n', '\r']);
        let kind = if n < protected_until || fence.is_some() {
            Border::None
        } else if text.starts_with("## ") || text == "##" {
            Border::H2
        } else if text.starts_with("### ") || text == "###" {
            Border::H3
        } else if prev_blank
            && !after_heading
            && !text.trim().is_empty()
            && !text.starts_with([' ', '\t'])
            && !text.starts_with("---")
            // A table stays with its caption, and a merge note with its table.
            && !text.starts_with('|')
            && !text.starts_with("Merged cells: ")
            && !text.ends_with("not shown (see the model JSON).")
        {
            Border::Blank
        } else {
            Border::None
        };
        kinds.push(kind);
        if n >= protected_until {
            match fence {
                None => {
                    if let Some((ch, run, _)) = fence_run(text) {
                        fence = Some((ch, run));
                    }
                }
                Some((ch, run)) => {
                    if let Some((c, r, rest)) = fence_run(text)
                        && c == ch
                        && r >= run
                        && rest.trim().is_empty()
                    {
                        fence = None;
                    }
                }
            }
        }
        prev_blank = text.trim().is_empty();
        if !prev_blank {
            after_heading = fence.is_none() && text.starts_with('#');
        }
    }
    (starts, kinds)
}

/// Splits `range` before every line whose border is in `at`.
fn split(
    range: (usize, usize),
    starts: &[usize],
    kinds: &[Border],
    at: &[Border],
) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut from = range.0;
    let first = starts.partition_point(|s| *s <= range.0);
    for (start, kind) in starts[first..].iter().zip(&kinds[first..]) {
        if *start >= range.1 {
            break;
        }
        if at.contains(kind) {
            out.push((from, *start));
            from = *start;
        }
    }
    out.push((from, range.1));
    out
}

pub(crate) fn paginate(md: &str, page_tokens: usize) -> Vec<String> {
    if page_tokens == 0 || tokens(md) <= page_tokens {
        return vec![md.to_string()];
    }
    let (starts, kinds) = borders(md);
    let fits = |r: &(usize, usize)| tokens(&md[r.0..r.1]) <= page_tokens;

    // Atomic pieces: whole sections where they fit, else their `###` parts, else blank-line
    // parts.
    let mut pieces: Vec<(usize, usize)> = Vec::new();
    for section in split((0, md.len()), &starts, &kinds, &[Border::H2]) {
        if fits(&section) {
            pieces.push(section);
            continue;
        }
        for part in split(section, &starts, &kinds, &[Border::H3]) {
            if fits(&part) {
                pieces.push(part);
            } else {
                pieces.extend(split(part, &starts, &kinds, &[Border::Blank]));
            }
        }
    }

    // Greedy packing on character counts, so each piece is counted once.
    let limit_chars = page_tokens.saturating_mul(4).saturating_add(3);
    let mut pages = Vec::new();
    let mut current: Option<(usize, usize, usize)> = None;
    for piece in pieces {
        let chars = md[piece.0..piece.1].chars().count();
        current = match current {
            None => Some((piece.0, piece.1, chars)),
            Some((from, _, have)) if have + chars <= limit_chars => {
                Some((from, piece.1, have + chars))
            }
            Some((from, to, _)) => {
                pages.push(md[from..to].to_string());
                Some((piece.0, piece.1, chars))
            }
        };
    }
    if let Some((from, to, _)) = current {
        pages.push(md[from..to].to_string());
    }
    pages
}
