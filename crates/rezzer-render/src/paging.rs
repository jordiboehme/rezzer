// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Splits rendered Markdown into pages at section borders.
//!
//! Pages break before a `## ` line. A section larger than a page breaks before its `### `
//! lines, and a part still too large at a blank line. Breaks never fall inside a fence, the
//! frontmatter or the title, never before an indented line (it would turn into code) and
//! never before a `---` line (it would look like frontmatter), and a heading stays with the
//! block after it. A table stays with what comes before it and its notes with it; a sheet
//! region's layout line stays with what comes before it and with the table or list after
//! it.
//!
//! Inside `## Logic` only (spec 8.7), a page that starts at a blank line begins with the
//! heading line of the `##` or `###` section it lies in and, inside a parameter group, with
//! the group's title line, each followed by ` (continued)`. Joining the pages without these
//! lines gives the input back unchanged; outside `## Logic` joining the pages gives the
//! input back as it is.
//!
//! The splitter cuts with the same borders ([`cut`]), against a size measure of its own, and
//! cuts a part still over its limit at line ends, whitespace and characters as a last resort
//! ([`hard_cut`]): those pieces end and begin with `(continued)` lines, a cut fence is closed
//! and opened again, and a cut table repeats its header.

use std::borrow::Cow;
use std::rc::Rc;

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
pub(crate) fn fence_run(line: &str) -> Option<(char, usize, &str)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let ch = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let run = rest.chars().take_while(|c| *c == ch).count();
    (run >= 3).then(|| (ch, run, &rest[run..]))
}

/// The starts of the row kind line under a sheet table.
const ROW_KINDS: [&str; 4] = ["Subtotal row", "Total row", "Group row", "Total column"];

/// The words that start a sheet region's layout line or lead line, before its range.
const REGION_WORDS: [&str; 4] = ["Table ", "Grid ", "Label and value pairs ", "Text "];

/// A cell address such as `B12` at the start of `text`: what follows it.
fn after_cell(text: &str) -> Option<&str> {
    let letters = text.bytes().take_while(u8::is_ascii_uppercase).count();
    let digits = text[letters..]
        .bytes()
        .take_while(u8::is_ascii_digit)
        .count();
    ((1..=3).contains(&letters) && (1..=7).contains(&digits)).then(|| &text[letters + digits..])
}

/// True for the layout line of a sheet region (`Table A1:F9, header row 1, ...`) or the
/// lead line of a label and value or text region (`Text A19:A25:`).
fn layout_line(text: &str) -> bool {
    REGION_WORDS.iter().any(|word| {
        let Some(rest) = text.strip_prefix(word).and_then(after_cell) else {
            return false;
        };
        let rest = match rest.strip_prefix(':').and_then(after_cell) {
            Some(after) => after,
            None => rest,
        };
        rest.starts_with([',', '.', ':']) || rest.starts_with(" (data ")
    })
}

/// The words that start the title line of a parameter group in `## Logic`.
const GROUP_TITLE: &str = "Parameters in ";

/// What a line stands in, inside `## Logic`: the line of the `##` or `###` section it lies
/// in (the innermost), and of the parameter group title it lies under, as line numbers.
#[derive(Debug, Clone, Copy, Default)]
struct Context {
    section: Option<usize>,
    group: Option<usize>,
}

/// The border before each line, each line's start offset and its context.
fn borders(md: &str) -> (Vec<usize>, Vec<Border>, Vec<Context>) {
    let lines: Vec<&str> = md.split_inclusive('\n').collect();
    let mut starts = Vec::with_capacity(lines.len());
    let mut kinds = Vec::with_capacity(lines.len());
    let mut contexts = Vec::with_capacity(lines.len());
    let mut offset = 0;
    // The open `##` and `###` heading lines, whether the `##` is `## Logic`, and the open
    // group title.
    let mut h2: Option<usize> = None;
    let mut h3: Option<usize> = None;
    let mut logic = false;
    let mut group: Option<usize> = None;
    // The last line that is not blank.
    let mut last_text: Option<usize> = None;

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
            // A sheet region's layout line stays with what comes before it, and its row
            // kinds with its table.
            && !layout_line(text)
            && !ROW_KINDS.iter().any(|k| text.starts_with(k))
            && !text.ends_with("not shown (see the model JSON).")
        {
            Border::Blank
        } else {
            Border::None
        };
        kinds.push(kind);
        if n >= protected_until && fence.is_none() {
            if kind == Border::H2 {
                (h2, h3, group) = (Some(n), None, None);
                logic = text == "## Logic";
            } else if kind == Border::H3 {
                (h3, group) = (Some(n), None);
            } else if text.starts_with('#') {
                group = None;
            } else if logic && prev_blank && text.starts_with(GROUP_TITLE) && text.ends_with(':') {
                group = Some(n);
            } else if prev_blank && text.starts_with('|') && last_text != group {
                // A table with no title line right before it starts a group without a title.
                group = None;
            }
        }
        contexts.push(Context {
            section: if logic { h3.or(h2) } else { None },
            group,
        });
        if !text.trim().is_empty() {
            last_text = Some(n);
        }
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
            // A layout line stays with the table or list under it, like a heading.
            after_heading = fence.is_none() && (text.starts_with('#') || layout_line(text));
        }
    }
    (starts, kinds, contexts)
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

/// The lines a page starting at `start` begins with: when it starts at a blank-line border
/// inside a section, the section's heading line, then the title of the group it starts
/// inside (none when it starts at the title), each marked continued.
fn continued(
    md: &str,
    start: usize,
    starts: &[usize],
    kinds: &[Border],
    contexts: &[Context],
) -> String {
    let Ok(n) = starts.binary_search(&start) else {
        return String::new();
    };
    if kinds[n] != Border::Blank {
        return String::new();
    }
    let line = |i: usize| {
        let end = starts.get(i + 1).copied().unwrap_or(md.len());
        md[starts[i]..end].trim_end_matches(['\n', '\r'])
    };
    let mut out = String::new();
    let context = contexts[n];
    if let Some(h) = context.section {
        out.push_str(&format!("{} (continued)\n\n", line(h)));
        if let Some(g) = context.group.filter(|&g| g != n) {
            out.push_str(&format!("{} (continued)\n\n", line(g)));
        }
    }
    out
}

pub(crate) fn paginate(md: &str, page_tokens: usize) -> Vec<String> {
    if page_tokens == 0 || tokens(md) <= page_tokens {
        return vec![md.to_string()];
    }
    // A page of `page_tokens` tokens holds up to 4 times as many characters, plus 3.
    let limit = page_tokens.saturating_mul(4).saturating_add(3);
    pages(md, limit, &|text: &str| text.chars().count(), false)
}

/// Cuts `md` into pieces whose `cost` is at most `budget`, at the borders [`paginate`] uses.
/// A part still over the budget at a blank line is cut at its line ends, then at whitespace,
/// then between characters ([`hard_cut`]). `cost` must add up: the cost of two texts joined
/// is the sum of their costs.
pub(crate) fn cut(md: &str, budget: usize, cost: &dyn Fn(&str) -> usize) -> Vec<String> {
    if cost(md) <= budget {
        return vec![md.to_string()];
    }
    pages(md, budget, cost, true)
}

/// A piece to pack: a part of the input, or a part [`hard_cut`] wrote.
enum Piece {
    Range(usize, usize),
    Owned(String),
}

/// Pages of at most `limit` by `size`; with `hard`, a blank-line part over the limit is cut
/// further ([`hard_cut`]), else it stays one page.
fn pages(md: &str, limit: usize, size: &dyn Fn(&str) -> usize, hard: bool) -> Vec<String> {
    let (starts, kinds, contexts) = borders(md);
    let fits = |r: &(usize, usize)| size(&md[r.0..r.1]) <= limit;

    // Atomic pieces: whole sections where they fit, else their `###` parts, else blank-line
    // parts.
    let mut pieces: Vec<Piece> = Vec::new();
    for section in split((0, md.len()), &starts, &kinds, &[Border::H2]) {
        if fits(&section) {
            pieces.push(Piece::Range(section.0, section.1));
            continue;
        }
        for part in split(section, &starts, &kinds, &[Border::H3]) {
            if fits(&part) {
                pieces.push(Piece::Range(part.0, part.1));
                continue;
            }
            for small in split(part, &starts, &kinds, &[Border::Blank]) {
                if hard && !fits(&small) {
                    pieces.extend(
                        hard_cut(&md[small.0..small.1], limit, size)
                            .into_iter()
                            .map(Piece::Owned),
                    );
                } else {
                    pieces.push(Piece::Range(small.0, small.1));
                }
            }
        }
    }

    // Greedy packing, so each piece is measured once. A page after the first counts the
    // lines it begins with.
    let mut pages = Vec::new();
    let mut current: Option<(String, usize)> = None;
    for piece in pieces {
        let (text, start) = match &piece {
            Piece::Range(from, to) => (&md[*from..*to], Some(*from)),
            Piece::Owned(text) => (text.as_str(), None),
        };
        let cost = size(text);
        current = match current {
            None => Some((text.to_string(), cost)),
            Some((mut page, have)) if have + cost <= limit => {
                page.push_str(text);
                Some((page, have + cost))
            }
            Some((page, _)) => {
                pages.push(page);
                let mut lead = start
                    .map(|s| continued(md, s, &starts, &kinds, &contexts))
                    .unwrap_or_default();
                if hard && size(&lead) > limit / 2 {
                    // A very long section or group title: its continued line is shortened,
                    // so the part still has room.
                    lead = shortened(&lead, limit / 8);
                }
                let lead_cost = size(&lead);
                if hard && lead_cost + cost > limit && lead_cost < limit {
                    // The lines the page begins with push it over: cut the part to the room
                    // they leave; its pieces after the first begin with `(continued)`.
                    let mut cut = hard_cut(text, limit - lead_cost, size).into_iter();
                    let first = format!("{lead}{}", cut.next().unwrap_or_default());
                    let rest: Vec<String> = cut.collect();
                    match rest.split_last() {
                        None => {
                            let have = size(&first);
                            Some((first, have))
                        }
                        Some((last, middle)) => {
                            pages.push(first);
                            pages.extend(middle.iter().cloned());
                            Some((last.clone(), size(last)))
                        }
                    }
                } else {
                    Some((lead + text, lead_cost + cost))
                }
            }
        };
    }
    if let Some((page, _)) = current {
        pages.push(page);
    }
    pages
}

/// The line a piece cut by [`hard_cut`] ends with, after a blank line, and the line the next
/// piece begins with.
const CONTINUED: &str = "(continued)";

/// What a line of a cut part stands in.
#[derive(Debug, Clone, Default)]
struct CutState {
    /// The open fence: its character, its run and its opener line.
    fence: Option<(char, usize, Rc<str>)>,
    /// The header and separator rows of the table the line stands in.
    table: Option<Rc<str>>,
    /// The open fence's opener takes more than half the limit: its lines are written as an
    /// indented code block instead, without opener and closer, so no piece repeats it.
    indented: bool,
    /// The line start of the open fence or math block: its quote markers and indent, which
    /// its closer and opener take too.
    prefix: Rc<str>,
    /// A `$$` math block is open.
    math: bool,
}

impl CutState {
    /// The fence closer a piece that ends here needs.
    fn closer(&self) -> String {
        if self.math {
            return format!("{}$$\n", self.prefix);
        }
        if self.indented {
            return String::new();
        }
        self.fence
            .as_ref()
            .map(|(ch, run, _)| format!("{}{}\n", self.prefix, ch.to_string().repeat(*run)))
            .unwrap_or_default()
    }
}

/// The start of a line that a piece of it keeps: quote markers (`>` with up to 3 spaces
/// before and one after) and then its indent; and the rest.
fn line_prefix(line: &str) -> (&str, &str) {
    let mut at = 0;
    loop {
        let rest = &line[at..];
        let spaces = rest.len() - rest.trim_start_matches(' ').len();
        if spaces <= 3 && rest[spaces..].starts_with('>') {
            at += spaces + 1;
            if line[at..].starts_with(' ') {
                at += 1;
            }
            continue;
        }
        at += spaces;
        return (&line[..at], &line[at..]);
    }
}

/// The continued lines of `lead` with each title cut to at most `max` characters.
fn shortened(lead: &str, max: usize) -> String {
    let suffix = format!(" {CONTINUED}");
    let mut out = String::with_capacity(lead.len().min(4 * max + 64));
    for line in lead.split("\n\n").filter(|l| !l.is_empty()) {
        let title = line.strip_suffix(suffix.as_str()).unwrap_or(line);
        out.push_str(&crate::split::capped(title, max.max(8)));
        out.push_str(&suffix);
        out.push_str("\n\n");
    }
    out
}

/// The line that stands for a fence written as an indented code block: its language, at
/// most 64 characters of it.
fn language_line(info: &str) -> Option<String> {
    const LINE: usize = 64;
    let escaped = crate::blocks::line_text(info.trim());
    // Cut after escaping, so the line is at most 64 characters, never inside an escape.
    let room = LINE - "Code in :".len();
    let mut cut: String = escaped.chars().take(room).collect();
    if cut.bytes().rev().take_while(|b| *b == b'\\').count() % 2 == 1 {
        cut.pop();
    }
    (!cut.is_empty()).then(|| format!("Code in {cut}:"))
}

/// A table separator row: only `|`, `-`, `:` and spaces, with a `-`.
fn separator_row(line: &str) -> bool {
    let line = line.trim_end_matches(['\n', '\r']);
    line.starts_with('|')
        && line.contains('-')
        && line.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// A line is cut to fill what is left of a piece only when at least this many characters are
/// left; else it begins a piece of its own.
const MIN_ROOM: usize = 64;

/// Ends a piece cut by [`hard_cut`]: an open fence closed, then a blank line and
/// `(continued)`.
fn close(cur: &mut String, state: &CutState, out: &mut Vec<String>) {
    if !cur.ends_with('\n') {
        cur.push('\n');
    }
    cur.push_str(&state.closer());
    cur.push_str(&format!("\n{CONTINUED}\n"));
    out.push(std::mem::take(cur));
}

/// Cuts one part over the limit, which has no border of its own, at line ends; a line over
/// the limit is cut at whitespace, then between characters, a table row cell by cell. Each
/// piece but the last ends with a blank line and `(continued)`, each but the first begins with
/// `(continued)` and a blank line. A cut fence is closed before that line and opened again
/// with the same opener; a cut table repeats its header and separator rows (when they take at
/// most half the limit). Every piece stays within `limit` by `size`, unless the limit cannot
/// hold even a few characters.
fn hard_cut(text: &str, limit: usize, size: &dyn Fn(&str) -> usize) -> Vec<String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let tail_cost = size(&format!("\n{CONTINUED}\n"));
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_cost = 0usize;
    let mut has_content = false;
    let mut state = CutState::default();

    // The lead of a piece after the first: the continued line, the fence opener and, before a
    // table row, the table's header.
    let lead = |state: &CutState, row: bool| -> String {
        let mut lead = format!("{CONTINUED}\n\n");
        if let Some((_, _, opener)) = &state.fence
            && !state.indented
        {
            lead.push_str(opener);
        }
        if state.math {
            lead.push_str(&format!("{}$$\n", state.prefix));
        }
        if row && let Some(header) = &state.table {
            lead.push_str(header);
        }
        lead
    };

    for (i, line) in lines.iter().enumerate() {
        // The state after this line, and whether it is a table row under a header.
        let mut after = state.clone();
        let content = line.trim_end_matches(['\n', '\r']);
        let mut row = false;
        // What this line is written as: itself, or in an indented code block (see
        // `CutState::indented`) its text indented and the fence lines as blank lines.
        let mut written: Cow<'_, str> = Cow::Borrowed(line);
        let (prefix, rest) = line_prefix(content);
        match &state.fence {
            None if state.math => {
                if rest.trim_end() == "$$" {
                    after.math = false;
                }
            }
            None => {
                if let Some((ch, run, info)) = fence_run(rest) {
                    after.fence = Some((ch, run, Rc::from(*line)));
                    after.prefix = Rc::from(prefix);
                    after.table = None;
                    if size(line) > limit / 2 {
                        after.indented = true;
                        let blank = prefix.trim_end();
                        written = Cow::Owned(match language_line(info) {
                            Some(l) => format!("\n{prefix}{l}\n{blank}\n"),
                            None => "\n".to_string(),
                        });
                    }
                } else if rest.trim_end() == "$$" {
                    after.math = true;
                    after.prefix = Rc::from(prefix);
                    after.table = None;
                } else if content.starts_with('|') {
                    if state.table.is_some() {
                        row = !separator_row(content);
                    } else if lines.get(i + 1).is_some_and(|next| separator_row(next)) {
                        let header = format!("{line}{}", lines[i + 1]);
                        if size(&header) <= limit / 2 {
                            after.table = Some(Rc::from(header));
                        }
                    }
                } else {
                    after.table = None;
                }
            }
            Some((ch, run, _)) => {
                if let Some((c, r, tail)) = fence_run(rest)
                    && c == *ch
                    && r >= *run
                    && tail.trim().is_empty()
                {
                    after.fence = None;
                    after.indented = false;
                    if state.indented {
                        written = Cow::Borrowed("\n");
                    }
                } else if state.indented && !rest.trim().is_empty() {
                    written = Cow::Owned(format!("{prefix}    {rest}\n"));
                }
            }
        }

        let mut work: Vec<String> = vec![written.into_owned()];
        let mut w = 0;
        while w < work.len() {
            let piece = std::mem::take(&mut work[w]);
            let cost = size(&piece);
            let reserve = tail_cost + size(&after.closer());
            // The size of a fresh piece's lead, only when it is needed.
            let fresh = || size(&lead(&state, row));
            if cur_cost + cost + reserve > limit
                && work.len() == 1
                && fresh() + cost + reserve > limit
            {
                let fresh = fresh();
                // The line is over the limit even of a fresh piece: cut it into lines, the
                // first filling this piece when it has room for some, the others a piece each.
                let mut first = limit.saturating_sub(cur_cost + reserve);
                if first < MIN_ROOM && has_content {
                    close(&mut cur, &state, &mut out);
                    cur = lead(&state, row);
                    cur_cost = fresh;
                    has_content = false;
                    first = limit.saturating_sub(cur_cost + reserve);
                }
                let room = limit.saturating_sub(fresh + reserve);
                let mut chunks = cut_line(&piece, first, room, &state, row, size);
                if has_content && chunks.first().is_some_and(|c| size(c) > first) {
                    // Too little room left here for a piece of this line.
                    close(&mut cur, &state, &mut out);
                    cur = lead(&state, row);
                    cur_cost = fresh;
                    has_content = false;
                    chunks = cut_line(&piece, room, room, &state, row, size);
                }
                if chunks.len() > 1 {
                    work = chunks;
                    w = 0;
                    continue;
                }
            }
            if cur_cost + cost + reserve > limit && has_content {
                close(&mut cur, &state, &mut out);
                cur = lead(&state, row);
                cur_cost = size(&cur);
            }
            cur.push_str(&piece);
            cur_cost += cost;
            has_content = true;
            w += 1;
        }
        state = after;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// One line over the limit cut into lines by `size`, the first within `first`, the others
/// within `room`, each ending with a newline: a table row (`row`) into full rows of the same
/// column count, the cut cell marked `(continued)`; any other line at whitespace, else
/// between characters. Outside a fence each line after the first loses its leading
/// whitespace and gets a block marker at its start escaped; inside an indented code block each
/// line keeps the indent. The rooms are halved until every line fits, at most 8 times.
fn cut_line(
    line: &str,
    first: usize,
    room: usize,
    state: &CutState,
    row: bool,
    size: &dyn Fn(&str) -> usize,
) -> Vec<String> {
    let content = line.trim_end_matches(['\n', '\r']);
    let fence = state
        .fence
        .as_ref()
        .map(|(ch, _, _)| *ch)
        .or(state.math.then_some('$'));
    if !row {
        // Each piece keeps the line's quote markers and indent (an indented code line its
        // indent).
        let (prefix, text) = line_prefix(content);
        let n = prefix.chars().count();
        // A prefix longer than a quarter of the room is not repeated: it would make the
        // pieces many and the output grow with it. The first piece keeps it, as before.
        if !prefix.is_empty() && n.saturating_mul(4) <= room {
            let (mut a, mut b) = (first.saturating_sub(n), room.saturating_sub(n));
            let mut lines = Vec::new();
            for _ in 0..=8 {
                lines = text_lines(text, a, b, fence);
                let fits = lines
                    .iter()
                    .enumerate()
                    .all(|(i, l)| size(l) + n <= if i == 0 { first } else { room });
                if fits {
                    break;
                }
                (a, b) = (a / 2, b / 2);
            }
            return lines.into_iter().map(|l| format!("{prefix}{l}")).collect();
        }
    }
    let (mut a, mut b) = (first, room);
    let mut best: Vec<String> = Vec::new();
    for _ in 0..=8 {
        let lines = if row {
            row_lines(content, a, b, size)
        } else {
            text_lines(content, a, b, fence)
        };
        let fits = lines
            .iter()
            .enumerate()
            .all(|(i, l)| size(l) <= if i == 0 { first } else { room });
        best = lines;
        if fits {
            break;
        }
        (a, b) = (a / 2, b / 2);
    }
    best
}

/// The byte offset after the first `n` characters of `text`; `None` when it has no more.
fn after_chars(text: &str, n: usize) -> Option<usize> {
    text.char_indices().nth(n).map(|(i, _)| i)
}

/// A cut at or before `end` in `text` (outside a fence: at the last whitespace in its second
/// half, else at `end`), never after an odd run of backslashes (it would escape what follows).
/// At least one character. Inside a fence a piece may begin with the fence's character: the
/// renderer makes every fence longer than any run inside, so such a piece never closes it.
fn cut_at(text: &str, end: usize, fence: Option<char>) -> usize {
    let mut cut = end;
    if fence.is_none()
        && let Some(space) = text[..end].rfind(char::is_whitespace)
        && space > end / 2
    {
        cut = space;
    }
    let backslashes = |at: usize| text[..at].bytes().rev().take_while(|b| *b == b'\\').count();
    while cut > 0 && backslashes(cut) % 2 == 1 {
        cut -= 1;
    }
    if cut == 0 {
        cut = text.chars().next().map_or(text.len(), char::len_utf8);
    }
    while !text.is_char_boundary(cut) {
        cut += 1;
    }
    cut
}

/// `content` in lines of at most `first` characters, then `chars`, the newline and an escape
/// included.
fn text_lines(content: &str, first: usize, chars: usize, fence: Option<char>) -> Vec<String> {
    let room = |n: usize| n.saturating_sub(2).max(1);
    let mut pieces: Vec<&str> = Vec::new();
    let mut rest = content;
    while let Some(end) = after_chars(rest, room(if pieces.is_empty() { first } else { chars })) {
        let cut = cut_at(rest, end, fence);
        pieces.push(&rest[..cut]);
        rest = &rest[cut..];
        if fence.is_none() {
            rest = rest.trim_start();
        }
    }
    if !rest.is_empty() || pieces.is_empty() {
        pieces.push(rest);
    }
    pieces
        .iter()
        .enumerate()
        .map(|(i, piece)| {
            let mut line = String::with_capacity(piece.len() + 2);
            if fence.is_some() {
                line.push_str(piece);
            } else if i == 0 {
                line.push_str(piece.trim_end());
            } else {
                crate::blocks::escape_line_start(piece.trim_end(), &mut line);
            }
            line.push('\n');
            line
        })
        .collect()
}

/// The cells of a table row, without the outer pipes; a `|` after an odd run of backslashes
/// is part of its cell.
fn row_cells(row: &str) -> Vec<&str> {
    let inner = row.trim();
    let inner = inner.strip_prefix('|').unwrap_or(inner);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    let mut cells = Vec::new();
    let mut from = 0;
    let mut backslashes = 0;
    for (i, b) in inner.bytes().enumerate() {
        match b {
            b'\\' => backslashes += 1,
            b'|' if backslashes % 2 == 0 => {
                cells.push(inner[from..i].trim());
                from = i + 1;
                backslashes = 0;
            }
            _ => backslashes = 0,
        }
    }
    cells.push(inner[from..].trim());
    cells
}

fn row_line(cells: &[String]) -> String {
    let mut line = String::from("|");
    for c in cells {
        line.push(' ');
        line.push_str(c);
        line.push_str(" |");
    }
    line.push('\n');
    line
}

/// A table row over the room as full rows of the same column count, the first within
/// `first` characters, the others within `chars`: the cells in order, a cell cut where the
/// row is full, its text ending with ` (continued)` and going on in the next row after
/// `(continued) `, the other cells of that row empty.
fn row_lines(
    content: &str,
    first: usize,
    chars: usize,
    size: &dyn Fn(&str) -> usize,
) -> Vec<String> {
    let cells = row_cells(content);
    let width = cells.len();
    let base = size(&row_line(&vec![String::new(); width]));
    let marks = format!(" {CONTINUED}").len() + format!("{CONTINUED} ").len();
    let cap_of = |n: usize| n.checked_sub(base + marks).filter(|c| *c >= 16);
    let Some(cap) = cap_of(chars) else {
        return text_lines(content, first, chars, None);
    };
    // Too little room for a row in the first line: the caller begins a fresh piece.
    let first_cap = cap_of(first).unwrap_or(cap);
    let mut out = Vec::new();
    let mut current = vec![String::new(); width];
    let mut room = first_cap;
    for (i, cell) in cells.iter().enumerate() {
        let mut rest: &str = cell;
        loop {
            let Some(end) = after_chars(rest, room) else {
                room -= rest.chars().count();
                current[i].push_str(rest);
                break;
            };
            let cut = cut_at(rest, end, None);
            current[i].push_str(rest[..cut].trim_end());
            current[i].push_str(&format!(" {CONTINUED}"));
            out.push(row_line(&current));
            current = vec![String::new(); width];
            current[i] = format!("{CONTINUED} ");
            room = cap;
            rest = rest[cut..].trim_start();
        }
    }
    out.push(row_line(&current));
    out
}

#[cfg(test)]
mod tests {
    use super::cut;

    #[test]
    fn a_continued_lead_never_pushes_a_piece_over_the_budget() {
        // The paragraph just under the budget starts a new piece, which begins with the
        // continued heading of its Logic part.
        let budget = 2_000;
        let words = "lorem ipsum ".repeat(200);
        let paragraph = words[..budget - 10].trim_end();
        let md = format!("## Logic\n\n### Data\n\nSpreadsheet formulas.\n\n{paragraph}\n");
        let size = |t: &str| t.chars().count();
        let pieces = cut(&md, budget, &size);
        assert!(pieces.len() > 1);
        assert!(
            pieces[1].starts_with("### Data (continued)"),
            "{}",
            pieces[1]
        );
        for p in &pieces {
            assert!(size(p) <= budget, "{} over {budget}: {p}", size(p));
        }
    }

    #[test]
    fn a_very_long_section_title_is_shortened_in_its_continued_line() {
        let budget = 2_000;
        let title = "Sheet with a title ".repeat(80);
        let words = "lorem ipsum ".repeat(200);
        let paragraph = words[..budget - 10].trim_end();
        let md = format!("## Logic\n\n### {title}\n\nSpreadsheet formulas.\n\n{paragraph}\n");
        let size = |t: &str| t.chars().count();
        let pieces = cut(&md, budget, &size);
        assert!(pieces.len() > 1);
        assert!(
            pieces[1].starts_with("### Sheet with a title"),
            "{}",
            pieces[1]
        );
        assert!(pieces[1].contains("... (continued)"));
        for p in &pieces {
            assert!(size(p) <= budget, "{} over {budget}", size(p));
        }
    }

    #[test]
    fn a_language_line_is_at_most_64_characters() {
        let line = super::language_line(&"*".repeat(200)).expect("a line");
        assert!(line.chars().count() <= 64, "{line}");
        assert!(line.ends_with(":") && !line.ends_with("\\:"));
    }

    #[test]
    fn quoted_lines_keep_their_markers_when_cut() {
        let line = format!("> {}\n", "lorem ipsum ".repeat(400));
        let size = |t: &str| t.chars().count();
        let pieces = cut(&line, 1_000, &size);
        assert!(pieces.len() > 2);
        for p in &pieces {
            for l in p.lines().filter(|l| l.contains("lorem")) {
                assert!(l.starts_with("> "), "{l}");
            }
        }
    }
}
