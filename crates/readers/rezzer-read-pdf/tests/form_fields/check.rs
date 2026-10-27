// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The invariants every read of a generated form is held to: no wrong pair ever, every value
//! once, a pair where the geometry is clear, a named loss where it is not, a joined column
//! where a column of totals stands beside its section, one note of each kind per page, and
//! for F3 an underlined title before its lines.

use std::collections::HashMap;

use rezzer_model::{Block, LossReason, ReadOutcome, Severity, plain_text};

use crate::common::{print_form, text};
use crate::shapes::Built;

/// Reads the file of one page.
pub(crate) fn read(bytes: &[u8]) -> ReadOutcome {
    crate::common::read(bytes)
}

/// Reads the pages `pages` as one file.
pub(crate) fn read_pages(pages: &[&Built]) -> ReadOutcome {
    let pages: Vec<_> = pages.iter().map(|b| b.page.clone()).collect();
    read(&print_form(&pages, "", ""))
}

/// The blocks of page `n` (from 0).
pub(crate) fn page_blocks(out: &ReadOutcome, n: usize) -> &[Block] {
    out.document
        .body
        .iter()
        .filter_map(|b| match b {
            Block::Section { blocks, .. } => Some(blocks.as_slice()),
            _ => None,
        })
        .nth(n)
        .unwrap_or(&[])
}

/// The blocks of the first page.
pub(crate) fn blocks(out: &ReadOutcome) -> &[Block] {
    page_blocks(out, 0)
}

/// The label and value pairs of the tables of two columns without a header row in `blocks`.
pub(crate) fn pairs_of(blocks: &[Block]) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for inner in blocks {
        if let Block::Table {
            rows,
            header_rows: 0,
            ..
        } = inner
        {
            for r in rows {
                if let [l, v] = r.as_slice() {
                    found.push((plain_text(&l.inlines), plain_text(&v.inlines)));
                }
            }
        }
    }
    found
}

/// An invariant a read of a generated form must hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Inv {
    /// Every pair is a right pair (ruling P6-X): never broken, not even by a known variant.
    RightPair,
    /// Every value stands once in the text: never broken either.
    ValueOnce,
    /// A field the geometry makes clear pairs, and a column of totals joins its section.
    ClearPairs,
    /// A field the geometry leaves unclear does not pair, and a loss names it.
    UnclearSaid,
    /// The underlined title is a heading followed by its lines, before the grid's table.
    Title,
    /// Each found-by note stands once on its page (ruling P6-W).
    NoteOnce,
}

/// One broken invariant, and what broke it.
#[derive(Debug)]
pub(crate) struct Broken(pub Inv, pub String);

/// A label as the read writes it without its raised marker.
fn bare<'a>(l: &'a str, b: &Built) -> &'a str {
    let mut l = l.trim();
    for m in &b.markers {
        if let Some(s) = l.strip_suffix(m) {
            l = s.trim_end();
        }
    }
    l
}

/// The fields' invariants on the first page.
pub(crate) fn check_fields(out: &ReadOutcome, b: &Built) -> Vec<Broken> {
    check_page(out, 0, b)
}

/// The invariants on page `n` (from 0) of a read whose page `b` was built: every pair is a
/// right pair, each clear field pairs (an empty one with `empty`), each unclear field does not
/// pair and a loss of the page names it, every value stands in the page's text as often as it
/// is listed, each column of totals joins its section, and each note stands once. Each broken
/// one is listed.
pub(crate) fn check_page(out: &ReadOutcome, n: usize, b: &Built) -> Vec<Broken> {
    let mut broken = Vec::new();
    let blocks = page_blocks(out, n);
    let pairs = pairs_of(blocks);
    for (l, v) in &pairs {
        match b.fields.iter().find(|f| f.0 == bare(l, b)) {
            Some((_, Ok(Some(want)))) if v == want => {}
            Some((_, Ok(None))) if v == "empty" => {}
            Some((_, want)) => broken.push(Broken(
                Inv::RightPair,
                format!("wrong pair {l:?}: {v:?}, want {want:?}"),
            )),
            // Pairs of other text (the title is checked on its own).
            None => {
                if l.contains("Hinweise") {
                    broken.push(Broken(
                        Inv::RightPair,
                        format!("the title is a pair: {l:?}: {v:?}"),
                    ));
                }
            }
        }
    }
    let page = format!("p{}", n + 1);
    let uncertain: Vec<&str> = out
        .document
        .losses
        .iter()
        .filter(|l| {
            l.why == LossReason::Uncertain && l.severity == Severity::Info && l.location == page
        })
        .map(|l| l.what.as_str())
        .collect();
    for (name, want) in &b.fields {
        let paired = pairs.iter().any(|(l, _)| bare(l, b) == *name);
        match want {
            Ok(_) if !paired => {
                broken.push(Broken(Inv::ClearPairs, format!("{name} does not pair")));
            }
            Err(()) if paired => broken.push(Broken(
                Inv::UnclearSaid,
                format!("{name} pairs though it is unclear"),
            )),
            Err(()) if !uncertain.iter().any(|w| w.contains(name)) => {
                broken.push(Broken(
                    Inv::UnclearSaid,
                    format!("no loss names {name}: {uncertain:?}"),
                ));
            }
            _ => {}
        }
    }
    for (title, values) in &b.columns {
        if !joined(blocks, title, values) {
            broken.push(Broken(
                Inv::ClearPairs,
                format!("the column {title} does not join its section with {values:?}"),
            ));
        }
    }
    let text = text(blocks);
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || c == '|')
        .filter(|w| !w.is_empty())
        .collect();
    // A value listed k times is drawn k times, and stands k times in the text.
    for (k, v) in b.values.iter().enumerate() {
        if b.values[..k].contains(v) {
            continue;
        }
        let drawn = b.values.iter().filter(|x| *x == v).count();
        let parts: Vec<&str> = v.split_whitespace().collect();
        let n = words
            .windows(parts.len())
            .filter(|w| *w == parts.as_slice())
            .count();
        if n != drawn {
            broken.push(Broken(
                Inv::ValueOnce,
                format!("{v:?} stands {n} times, drawn {drawn} times"),
            ));
        }
    }
    let mut notes: HashMap<String, usize> = HashMap::new();
    for bl in blocks {
        if let Block::Paragraph {
            inlines,
            role: Some(role),
            ..
        } = bl
            && role == "found-by"
        {
            *notes.entry(plain_text(inlines)).or_default() += 1;
        }
    }
    for (note, k) in notes {
        if k > 1 {
            broken.push(Broken(Inv::NoteOnce, format!("{note:?} stands {k} times")));
        }
    }
    broken
}

/// Whether a table of `blocks` heads a column with `title` whose cells under it are `values`.
fn joined(blocks: &[Block], title: &str, values: &[&str]) -> bool {
    blocks.iter().any(|b| {
        let Block::Table { rows, .. } = b else {
            return false;
        };
        let Some(c) = rows
            .first()
            .and_then(|r| r.iter().position(|c| plain_text(&c.inlines) == title))
        else {
            return false;
        };
        let below: Vec<String> = rows[1..]
            .iter()
            .filter_map(|r| r.get(c).map(|c| plain_text(&c.inlines)))
            .filter(|t| !t.is_empty())
            .collect();
        below == values
    })
}

/// The title's invariants: no table row holds it, it is a heading followed directly by its
/// two lines, and it stands before the grid's table.
pub(crate) fn check_title(out: &ReadOutcome) -> Vec<Broken> {
    let mut broken = Vec::new();
    let bl = blocks(out);
    let heading = bl.iter().position(
        |b| matches!(b, Block::Heading { inlines, .. } if plain_text(inlines) == "Hinweise Muster"),
    );
    let Some(h) = heading else {
        broken.push(Broken(Inv::Title, "the title is no heading".into()));
        return broken;
    };
    let next: String = bl[h + 1..]
        .iter()
        .take(2)
        .map(|b| match b {
            Block::Paragraph { inlines, .. } => plain_text(inlines),
            Block::List { .. } => "(a list)".into(),
            _ => "(other)".into(),
        })
        .collect::<Vec<_>>()
        .join(" | ");
    if !(next.contains("Muster vom 01.02.21 Kst.") && next.contains("- Musterhinweis 2 Muster")) {
        broken.push(Broken(
            Inv::Title,
            format!("the title's lines do not follow it: {next:?}"),
        ));
    }
    let grid = bl.iter().position(|b| {
        matches!(b, Block::Table { rows, .. }
            if rows.iter().flatten().any(|c| plain_text(&c.inlines) == "Gehalt"))
    });
    if grid.is_some_and(|t| t < h) {
        broken.push(Broken(
            Inv::Title,
            "the title stands after the grid's table".into(),
        ));
    }
    broken
}

/// A known break: the variant's name, the one invariant it breaks, and why.
pub(crate) type Known = (&'static str, Inv, &'static str);

/// Holds each read to the invariants, with `known` the variants that break one: the names
/// of the variants that break another one or break none though they are known.
pub(crate) fn unexpected(reads: Vec<(String, Vec<Broken>)>, known: &[Known]) -> Vec<String> {
    assert!(
        known
            .iter()
            .all(|k| k.1 != Inv::RightPair && k.1 != Inv::ValueOnce),
        "a wrong pair or a value lost is never a known break"
    );
    let mut out = Vec::new();
    for (name, broken) in reads {
        let held = match known.iter().find(|k| k.0 == name) {
            Some(k) => !broken.is_empty() && broken.iter().all(|b| b.0 == k.1),
            None => broken.is_empty(),
        };
        if !held {
            let list: Vec<String> = broken
                .iter()
                .map(|b| format!("{:?}: {}", b.0, b.1))
                .collect();
            out.push(format!("{name}: {list:#?}"));
        }
    }
    out
}

/// Reads `b` and asserts the fields' invariants.
pub(crate) fn assert_fields(b: &Built) {
    let out = read_pages(&[b]);
    let broken = check_fields(&out, b);
    assert!(
        broken.is_empty(),
        "{broken:#?}\n{:#?}\n{:#?}",
        blocks(&out),
        out.document.losses
    );
}
