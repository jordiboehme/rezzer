// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The title of a page: the largest heading in its head (see [`page_title`]).

use std::ops::Range;

use super::heading_block;
use crate::extract::text::address_shaped;
use crate::file::{Levels, plain_size};
use crate::line::{EDGE_SIZES, GAP_RATIO, LineRun, SIZE_SLACK_SHARE, TextLine, ink, size_key};

/// What `title_from` says of a title [`page_title`] found.
pub const TITLE_FROM: &str = "largest heading in the top third of page 1";

/// A title holds at most this many characters.
const TITLE_CHARS: usize = 120;

/// The head of a page: the top third of its height.
pub const HEAD_SHARE: f32 = 2.0 / 3.0;

/// The title of a page: the lines of its heading and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Title {
    /// The heading's lines, as indexes into the lines [`page_title`] was given.
    pub lines: Range<usize>,
    /// Its words, at most [`TITLE_CHARS`] characters, cut at a word.
    pub text: String,
}

/// A heading that may be the title: its size key, whether all its ink is bold, its top and
/// its text.
struct Candidate {
    lines: Range<usize>,
    size: i32,
    bold: bool,
    top: f32,
    text: String,
}

impl Candidate {
    /// Whether it wins over `other`: larger, else bold where `other` is not, else higher.
    fn beats(&self, other: &Candidate) -> bool {
        (self.size, self.bold, self.top) > (other.size, other.bold, other.top)
    }
}

/// The title of a page, from its lines in reading order: among its headings (each with the
/// lines `levels` gives a level and that go on one another, as the layout joins them, see
/// [`heading_block`]) whose top lies in the top third of the page (`height`), the one of the
/// largest size wins, whatever its level; on a tie the one whose ink is all bold, then the
/// highest. A heading the layout writes as a paragraph is none. A heading a little larger than
/// the body that opens an address is skipped (see [`opens_address`]), and so is the title of a
/// table of contents or an index (see [`CONTENTS_TITLES`]). At most [`TITLE_CHARS`]
/// characters, cut at a word.
pub fn page_title<R: LineRun>(
    lines: &[TextLine<'_, R>],
    levels: &Levels<'_>,
    height: f32,
) -> Option<Title> {
    let body_size = levels.body_size;
    if height.is_nan() || height <= 0.0 {
        return None;
    }
    let mut best: Option<Candidate> = None;
    let mut i = 0;
    while i < lines.len() {
        let first = &lines[i];
        let level = (!first.text.is_empty())
            .then(|| levels.level(first))
            .flatten();
        let Some(level) = level else {
            i += 1;
            continue;
        };
        let (j, demoted) = heading_block(lines, i, level, levels);
        let block = &lines[i..j];
        let top = first.y + first.h;
        let small = plain_size(first.size, body_size);
        let words: Vec<&str> = block
            .iter()
            .flat_map(|l| l.text.split_whitespace())
            .collect();
        if demoted.is_none()
            && top >= height * HEAD_SHARE
            && !(small && opens_address(lines, i, j))
            && !contents_title(&words)
        {
            let candidate = Candidate {
                lines: i..j,
                size: size_key(first.size),
                bold: block
                    .iter()
                    .flat_map(|l| l.runs.iter())
                    .filter(|r| ink(r.text()) > 0)
                    .all(LineRun::bold),
                top,
                text: words.join(" "),
            };
            if best.as_ref().is_none_or(|b| candidate.beats(b)) {
                best = Some(candidate);
            }
        }
        i = j;
    }
    best.map(|b| Title {
        lines: b.lines,
        text: capped(&b.text),
    })
    .filter(|t| !t.text.is_empty())
}

/// Titles of a table of contents or an index, accent folded (see [`fold`]), a list per
/// language (German, English, French, Italian or Portuguese, Spanish): such a heading names a
/// part of the document, not the document.
const CONTENTS_TITLES: [&str; 10] = [
    "contents",
    "table of contents",
    "inhalt",
    "inhaltsverzeichnis",
    "sommaire",
    "table des matieres",
    "indice",
    "contenido",
    "contenuto",
    "index",
];

/// Whether the words of a heading are a title of [`CONTENTS_TITLES`], a colon after it or
/// not.
fn contents_title(words: &[&str]) -> bool {
    let text = fold(&words.join(" "));
    let text = text.strip_suffix(':').unwrap_or(&text).trim_end();
    CONTENTS_TITLES.contains(&text)
}

/// `text` in small letters with its accents folded: an umlaut as its two letters (`ä` as
/// `ae`), `ß` as `ss`, another letter with an accent or a Polish mark as the letter (`è` as
/// `e`, `ś` as `s`).
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            '\u{e4}' => out.push_str("ae"),
            '\u{f6}' => out.push_str("oe"),
            '\u{fc}' => out.push_str("ue"),
            '\u{df}' => out.push_str("ss"),
            '\u{e0}'..='\u{e5}' => out.push('a'),
            '\u{e7}' => out.push('c'),
            '\u{e8}'..='\u{eb}' => out.push('e'),
            '\u{ec}'..='\u{ef}' => out.push('i'),
            '\u{f1}' => out.push('n'),
            '\u{f2}'..='\u{f5}' | '\u{f8}' => out.push('o'),
            '\u{f9}'..='\u{fb}' => out.push('u'),
            '\u{fd}' | '\u{ff}' => out.push('y'),
            '\u{105}' => out.push('a'),
            '\u{107}' => out.push('c'),
            '\u{119}' => out.push('e'),
            '\u{142}' => out.push('l'),
            '\u{144}' => out.push('n'),
            '\u{15b}' => out.push('s'),
            '\u{17a}' | '\u{17c}' => out.push('z'),
            c => out.push(c),
        }
    }
    out
}

/// `text` cut to at most [`TITLE_CHARS`] characters, at the last space within them when there
/// is one.
fn capped(text: &str) -> String {
    if text.chars().count() <= TITLE_CHARS {
        return text.to_string();
    }
    let cut: String = text.chars().take(TITLE_CHARS + 1).collect();
    match cut.rfind(' ') {
        Some(at) if at > 0 => cut[..at].trim_end().to_string(),
        _ => cut.chars().take(TITLE_CHARS).collect(),
    }
}

/// The lines a heading opens an address with, its own counted, are at most this many (see
/// [`opens_address`]).
const TITLE_ADDRESS_LINES: usize = 8;

/// Whether the heading of `lines[i..j]` opens an address: its lines share one left edge and,
/// with some of the lines right below it at that edge (each right below the one before, see
/// [`right_below`], and not larger than the heading), have the shape of an address (see
/// [`address_shaped`]): the heading and the first lines below, three or more in all and at
/// most [`TITLE_ADDRESS_LINES`]. The lines below may be smaller than the heading: a name in bold
/// over its street and town; a longer line after the town does not hide the address.
fn opens_address<R: LineRun>(lines: &[TextLine<'_, R>], i: usize, j: usize) -> bool {
    let first = &lines[i];
    let at_edge =
        |l: &TextLine<'_, R>| (l.x - first.x).abs() <= EDGE_SIZES * first.size.max(l.size);
    if !lines[i..j].iter().all(at_edge) {
        return false;
    }
    let mut k = j;
    while k < lines.len()
        && k - i < TITLE_ADDRESS_LINES
        && at_edge(&lines[k])
        && right_below(&lines[k - 1], &lines[k])
        && lines[k].size <= first.size * (1.0 + SIZE_SLACK_SHARE)
    {
        k += 1;
    }
    let texts: Vec<&str> = lines[i..k].iter().map(|l| l.text.as_str()).collect();
    (3..=texts.len()).any(|n| address_shaped(&texts[..n]))
}

/// Whether `cur` stands right below `prev`: under it, with a gap of at most [`GAP_RATIO`]
/// line heights, whatever their sizes.
pub fn right_below<R>(prev: &TextLine<'_, R>, cur: &TextLine<'_, R>) -> bool {
    let gap = prev.y - cur.y;
    gap > 0.0 && gap <= GAP_RATIO * prev.h.max(cur.h).max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accents_fold() {
        assert_eq!(fold("Table des Mati\u{e8}res"), "table des matieres");
        assert_eq!(fold("G\u{fc}ltig bis"), "gueltig bis");
    }

    #[test]
    fn long_titles_are_cut_at_a_word() {
        let long = "Wort ".repeat(30);
        let cut = capped(long.trim());
        assert!(cut.chars().count() <= TITLE_CHARS, "{cut}");
        assert!(cut.ends_with("Wort"), "{cut}");
        let one = "\u{e4}".repeat(130);
        assert_eq!(capped(&one).chars().count(), TITLE_CHARS);
    }
}
