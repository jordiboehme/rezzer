// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Annotations in the model, each at its passage.
//!
//! Each annotation stands at the block that holds the first glyph of its quote (`quote.rs`),
//! else at the block nearest to the top of its rectangle, else at its page. Replies nest
//! under their parent; members of a group join their parent. Comments become plain inlines,
//! line by line: nothing from the file is read as Markdown. Authors and comments copied into
//! the model are charged to the bytes the file may keep; past it what was read stays and one
//! loss names the rest.

use std::collections::HashMap;

use pdfparts::Annot;
use rezzer_limits::Budget;
use rezzer_model::{Anchor, Annotation, Inline, LossReason, Severity, link_target_written_len};

use crate::Out;
use crate::layout::LinkRoom;

/// What one annotation costs in the model and while it is placed, besides its texts, as
/// measured: the annotation with its lists, its place and its node in the reply tree.
const ANNOT_COST: u64 = 512;

/// The share of the memory limit the annotations may keep at most, besides what the pages
/// leave: the renderer and the JSON writer copy what is kept about four times over.
const OWN_SHARE: u64 = 64;

/// Reply levels kept as nested replies; deeper replies stay at the deepest level, so the
/// rendered lists and the cached model stay within their nesting limits.
const REPLY_DEPTH: usize = 16;

/// Glyphs and runs visited between two looks at the clock.
const POLL: usize = 4096;

/// Words for the subtypes, as the model names annotations.
fn kind_word(subtype: &str) -> String {
    match subtype {
        "Highlight" => "highlight",
        "Underline" => "underline",
        "StrikeOut" => "strikeout",
        "Squiggly" => "squiggly",
        "Text" => "note",
        "FreeText" => "free-text",
        "Ink" => "ink",
        "Caret" => "caret",
        "Stamp" => "stamp",
        "Square" => "square",
        "Circle" => "circle",
        "Line" => "line",
        "Polygon" => "polygon",
        "FileAttachment" => "file-attachment",
        "Sound" => "sound",
        "Redact" => "redact",
        other => return other.to_lowercase(),
    }
    .to_string()
}

/// Review states that close the annotation they answer.
fn closes(state: &str) -> bool {
    matches!(state, "Accepted" | "Completed" | "Cancelled")
}

/// Where each annotation of the file stands and what it quotes, once its page is laid out.
#[derive(Debug, Clone, Default)]
pub(crate) struct Placed {
    pub anchor: Option<Anchor>,
    pub quoted: Option<String>,
    /// The quote is made of whole runs, not of the glyphs inside the quads.
    pub from_runs: bool,
    /// A bound left the quote out, which its own loss says.
    pub cut: bool,
}

/// The comment of an annotation as inlines: one plain text per line, with line breaks.
fn body(contents: Option<&str>) -> Vec<Inline> {
    let mut out = Vec::new();
    let Some(text) = contents else {
        return out;
    };
    for (k, line) in text
        .split("\r\n")
        .flat_map(|l| l.split(['\r', '\n']))
        .enumerate()
    {
        if k > 0 {
            out.push(Inline::LineBreak);
        }
        if !line.is_empty() {
            out.push(Inline::plain(line));
        }
    }
    while matches!(out.last(), Some(Inline::LineBreak)) {
        out.pop();
    }
    out
}

/// When an annotation was last changed, else made, in seconds since 1970 UTC; the earliest
/// time there is when it has no date that parses.
fn when(a: &Annot) -> i64 {
    fn iso(d: &Option<pdfparts::PdfDate>) -> Option<&str> {
        d.as_ref().and_then(|d| d.iso.as_deref())
    }
    iso(&a.modified)
        .or_else(|| iso(&a.created))
        .and_then(seconds)
        .unwrap_or(i64::MIN)
}

/// Seconds since 1970 UTC of an ISO 8601 date as pdfparts writes it: `YYYY`, then `-MM`,
/// `-DD`, `THH:MM:SS` as far as given, then `Z` or an offset `+hh:mm`. Without an offset
/// the time counts as UTC.
fn seconds(iso: &str) -> Option<i64> {
    let b = iso.as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        let part = iso.get(from..from + len)?;
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    let year = num(0, 4)?;
    let field = |at: usize, sep: u8, default: i64| match b.get(at) {
        Some(&c) if c == sep => num(at + 1, 2),
        _ => Some(default),
    };
    let month = field(4, b'-', 1)?;
    let day = field(7, b'-', 1)?;
    let hour = field(10, b'T', 0)?;
    let minute = field(13, b':', 0)?;
    let second = field(16, b':', 0)?;
    let offset = match b.get(19) {
        Some(&c @ (b'+' | b'-')) => {
            let sign = if c == b'-' { -1 } else { 1 };
            sign * (num(20, 2)? * 3600 + num(23, 2).unwrap_or(0) * 60)
        }
        _ => 0,
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// The date: when it was last changed, else when it was made, in ISO 8601.
fn date(a: &Annot) -> Option<String> {
    let iso = |d: &Option<pdfparts::PdfDate>| d.as_ref().and_then(|d| d.iso.clone());
    iso(&a.modified).or_else(|| iso(&a.created))
}

/// Puts every annotation of the file in the model, replies under their parent. `placed`
/// holds where each stands and what it quotes, by its index in `annots`.
pub(crate) fn read(
    annots: &[Annot],
    placed: Vec<Placed>,
    budget: &Budget,
    room: &LinkRoom,
    out: &mut Out,
) {
    if annots.is_empty() {
        return;
    }
    let n = annots.len();
    // Each annotation and what it costs, built in file order; the first that does not fit
    // ends the list.
    let mut built: Vec<Option<Annotation>> = Vec::with_capacity(n);
    let mut cut: Option<(usize, String)> = None;
    let mut own = budget.limits().max_memory_bytes / OWN_SHARE;
    for (i, (a, p)) in annots.iter().zip(placed).enumerate() {
        if i % POLL == 0
            && let Err(hit) = budget.check_time()
        {
            cut = Some((i + 1, hit.to_string()));
            break;
        }
        let mut b = body(a.contents.as_deref());
        if p.from_runs && p.quoted.is_some() {
            if !b.is_empty() {
                b.push(Inline::plain(" "));
            }
            b.push(Inline::plain("(quote from whole text runs)"));
        }
        let kind = kind_word(&a.subtype);
        let date = date(a);
        let anchor = p.anchor.unwrap_or_else(|| Anchor(format!("p{}", a.page)));
        let resolved = a.state.as_deref().filter(|s| closes(s)).map(|_| true);
        // The quote was charged to the room when it was read; it counts toward the share.
        let bytes = [
            kind.as_str(),
            a.author.as_deref().unwrap_or(""),
            date.as_deref().unwrap_or(""),
            anchor.0.as_str(),
            a.contents.as_deref().unwrap_or(""),
        ]
        .iter()
        .map(|t| link_target_written_len(t))
        .fold(ANNOT_COST, u64::saturating_add);
        let share = bytes.saturating_add(p.quoted.as_deref().map_or(0, link_target_written_len));
        if share > own {
            cut = Some((
                i + 1,
                format!(
                    "the copies take more than the {own} bytes left of what the annotations may \
                     keep (a 64th of the memory limit)"
                ),
            ));
            break;
        }
        if bytes > room.left() {
            cut = Some((
                i + 1,
                format!(
                    "the copies take more than the {} bytes left of what the file may keep",
                    room.left()
                ),
            ));
            break;
        }
        if let Err(hit) = budget.add_elements(1 + b.len() as u64) {
            cut = Some((i + 1, hit.to_string()));
            break;
        }
        let taken = room.take_kept(bytes);
        debug_assert!(taken, "checked above");
        own -= share;
        built.push(Some(Annotation {
            kind,
            author: a.author.as_deref().map(str::to_string),
            date,
            anchor: Some(anchor),
            quoted: p.quoted,
            body: b,
            replies: Vec::new(),
            resolved,
        }));
    }
    let kept = built.len();
    let tree = nest(&annots[..kept]);
    out.doc.facets.annotations = tree.build(&mut built, &annots[..kept]);
    if let Some((first, why)) = cut {
        let what = if first == n {
            format!("annotation {first} is")
        } else {
            format!("annotations {first} to {n} are")
        };
        out.loss(
            "document",
            format!("{what} not kept: {why}"),
            LossReason::Limit,
            Severity::Warning,
        );
    }
    if tree.loops > 0 {
        let k = tree.loops;
        out.loss(
            "document",
            format!(
                "{k} {} in a loop of replies; {} listed without {} parent",
                if k == 1 {
                    "annotation replies"
                } else {
                    "annotations reply"
                },
                if k == 1 { "it is" } else { "they are" },
                if k == 1 { "its" } else { "their" },
            ),
            LossReason::Damaged,
            Severity::Warning,
        );
    }
    if tree.flattened > 0 {
        let k = tree.flattened;
        out.loss(
            "document",
            format!(
                "{k} {} deeper than {REPLY_DEPTH} levels {} at level {REPLY_DEPTH}",
                if k == 1 { "reply" } else { "replies" },
                if k == 1 { "is shown" } else { "are shown" },
            ),
            LossReason::Unsupported,
            Severity::Info,
        );
    }
}

/// The reply tree of a list of annotations.
struct Tree {
    /// Children of each annotation, in file order, and the annotations without a parent.
    children: Vec<Vec<usize>>,
    roots: Vec<usize>,
    /// Members of a group, which join their parent instead of replying to it.
    group: Vec<bool>,
    loops: usize,
    flattened: usize,
}

/// Who replies to whom. A reply whose parent is not listed stands alone; a loop of replies is
/// broken where the walk meets it again; a reply deeper than [`REPLY_DEPTH`] hangs from the
/// ancestor at that depth. Every walk is a loop, so a long chain cannot overflow the stack.
fn nest(annots: &[Annot]) -> Tree {
    let n = annots.len();
    let by_id: HashMap<&str, usize> = annots
        .iter()
        .enumerate()
        .map(|(i, a)| (a.id.as_str(), i))
        .collect();
    let mut parent: Vec<Option<usize>> = annots
        .iter()
        .enumerate()
        .map(|(i, a)| {
            a.in_reply_to
                .as_deref()
                .and_then(|p| by_id.get(p).copied())
                .filter(|&p| p != i)
        })
        .collect();
    let group: Vec<bool> = annots
        .iter()
        .map(|a| a.reply_type.as_deref() == Some("Group"))
        .collect();
    // Loops are broken first. 0: not seen, 1: on the walk now, 2: done.
    let mut state = vec![0u8; n];
    let mut loops = 0;
    let mut path: Vec<usize> = Vec::new();
    for start in 0..n {
        let mut at = start;
        while state[at] == 0 {
            state[at] = 1;
            path.push(at);
            match parent[at] {
                Some(p) if state[p] == 1 => {
                    // The reply that closes the loop loses its parent.
                    parent[at] = None;
                    loops += 1;
                    break;
                }
                Some(p) => at = p,
                None => break,
            }
        }
        for i in path.drain(..) {
            state[i] = 2;
        }
    }
    // Then the depths, walking up to the first annotation whose depth is known.
    let mut depth = vec![0usize; n];
    // The ancestor at the deepest level that keeps replies, for annotations at or below it.
    let mut top: Vec<usize> = (0..n).collect();
    let mut known = vec![false; n];
    for start in 0..n {
        let mut at = start;
        while !known[at] {
            path.push(at);
            match parent[at] {
                Some(p) => at = p,
                None => break,
            }
        }
        while let Some(i) = path.pop() {
            depth[i] = match parent[i] {
                Some(p) if known[p] => depth[p] + 1,
                _ => 0,
            };
            if depth[i] >= REPLY_DEPTH
                && let Some(p) = parent[i]
            {
                top[i] = top[p];
            }
            known[i] = true;
        }
    }
    let mut flattened = 0;
    // Deep replies hang from their ancestor at the deepest level kept.
    let mut hang: Vec<Option<usize>> = parent.clone();
    for i in 0..n {
        if depth[i] > REPLY_DEPTH
            && let Some(p) = parent[i]
        {
            hang[i] = Some(top[p]);
            flattened += 1;
        }
    }
    let mut children = vec![Vec::new(); n];
    let mut roots = Vec::new();
    for (i, p) in hang.iter().enumerate() {
        match p {
            Some(p) => children[*p].push(i),
            None => roots.push(i),
        }
    }
    Tree {
        children,
        roots,
        group,
        loops,
        flattened,
    }
}

impl Tree {
    /// The annotations as a tree: each root with its replies. Members of a group add their
    /// comment and replies to their parent; a reply that closes its parent marks it
    /// resolved.
    fn build(&self, built: &mut [Option<Annotation>], annots: &[Annot]) -> Vec<Annotation> {
        let mut out = Vec::with_capacity(self.roots.len());
        for &r in &self.roots {
            if let Some(a) = self.node(r, built, annots, 0) {
                out.push(a);
            }
        }
        out
    }

    /// One annotation with its replies. The depth of the tree is at most [`REPLY_DEPTH`]
    /// plus one, so the recursion is bounded.
    fn node(
        &self,
        i: usize,
        built: &mut [Option<Annotation>],
        annots: &[Annot],
        depth: usize,
    ) -> Option<Annotation> {
        let mut a = built[i].take()?;
        let mut latest: Option<((i64, usize), bool)> = None;
        // Group members join here, with their own members and replies.
        let mut todo: Vec<usize> = self.children[i].iter().rev().copied().collect();
        while let Some(c) = todo.pop() {
            if self.group[c] && depth <= REPLY_DEPTH {
                if let Some(member) = built[c].take() {
                    if !member.body.is_empty() {
                        if !a.body.is_empty() {
                            a.body.push(Inline::LineBreak);
                        }
                        a.body.extend(member.body);
                    }
                    if a.quoted.is_none() {
                        a.quoted = member.quoted;
                    }
                    a.resolved = a.resolved.or(member.resolved);
                }
                todo.extend(self.children[c].iter().rev());
                continue;
            }
            // The latest review state counts: by date, else by place in the file.
            if let Some(state) = annots[c].state.as_deref() {
                let key = (when(&annots[c]), c);
                if latest.as_ref().is_none_or(|(k, _)| key > *k) {
                    latest = Some((key, closes(state)));
                }
            }
            if let Some(reply) = self.node(c, built, annots, depth + 1) {
                a.replies.push(reply);
            }
        }
        if let Some((_, closed)) = latest {
            a.resolved = closed.then_some(true);
        }
        Some(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_compare_across_offsets() {
        assert_eq!(seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(seconds("1970-01-01T01:00:00+01:00"), Some(0));
        assert_eq!(seconds("1969-12-31T23:00:00-01:00"), Some(0));
        assert_eq!(seconds("2024-03-01"), Some(1_709_251_200));
        assert_eq!(seconds("x"), None);
    }

    #[test]
    fn comments_keep_their_lines() {
        assert_eq!(
            body(Some("a\r\nb\rc\n")),
            [
                Inline::plain("a"),
                Inline::LineBreak,
                Inline::plain("b"),
                Inline::LineBreak,
                Inline::plain("c")
            ]
        );
        assert!(body(None).is_empty());
    }
}
