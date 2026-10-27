// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The invariants every variant is held to, and the run of a shape's variants against its
//! pinned known breaks.

use std::collections::{BTreeSet, HashMap};

use rezzer_model::{Block, ReadOutcome, plain_text};

use crate::axes::{Page, Shape, Sheet, page_of, variants};
use crate::check::{self, page_blocks};
use crate::common::{needing, print_form, text};
use crate::known::{KNOWN, Known};

/// An invariant of a variant's read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Inv {
    /// Every pair is a right pair (ruling P6-X).
    RightPair,
    /// Every value stands once in the page's text (P6-X).
    ValueOnce,
    /// A found title, date or code is never another one than the page shows (P6-X).
    WrongValue,
    /// A field the layout makes clear pairs; a column of totals joins its section.
    ClearPairs,
    /// A field the layout leaves unclear does not pair, and a loss names it.
    UnclearSaid,
    /// The underlined title of F3 is a heading before its lines and the grid.
    F3Title,
    /// Each found-by note stands once on its page.
    NoteOnce,
    /// Each expected table stands on the page with exactly its rows.
    Table,
    /// Each expected heading is a heading; no unwanted text is one.
    Heading,
    /// Each expected paragraph of a role (a footnote, a running footer) stands on the page.
    Role,
    /// Each expected paragraph stands on the page with its lines.
    Paragraph,
    /// Texts that belong to different blocks are not read into one block.
    Apart,
    /// The document title is found.
    DocTitle,
    /// The document date is found.
    DocDate,
    /// Each code is found once.
    Code,
    /// The page needs vision exactly when it should.
    Vision,
    /// Each expected loss is said.
    LossSaid,
    /// The read equals the read of the same variant at scale 1 in the real sizes and fonts.
    SameReading,
    /// A variant whose read at scale 1 in the real sizes and fonts has no loss has none.
    NoLoss,
}

impl Inv {
    /// The invariants of ruling P6-X: a break is a wrong value, never only a lost one.
    pub(crate) fn wrong(self) -> bool {
        matches!(self, Inv::RightPair | Inv::ValueOnce | Inv::WrongValue)
    }
}

/// One broken invariant, and what broke it.
#[derive(Debug, Clone)]
pub(crate) struct Broken(pub Inv, pub String);

/// The file of a variant's pages.
pub(crate) fn file(page: &Page) -> Vec<u8> {
    let pages: Vec<_> = page.sheets.iter().map(|s| s.built.page.clone()).collect();
    print_form(&pages, "", "")
}

/// Reads a variant's file. A table's caption is written as a paragraph of role `caption`
/// right before it, so that its text is checked and compared as every other text is.
pub(crate) fn read(page: &Page) -> ReadOutcome {
    let mut out = crate::common::read(&file(page));
    out.document.body = hoist(std::mem::take(&mut out.document.body));
    out
}

/// `blocks` with each table caption moved into a paragraph before its table.
fn hoist(blocks: Vec<Block>) -> Vec<Block> {
    let mut out = Vec::with_capacity(blocks.len());
    for b in blocks {
        match b {
            Block::Section {
                kind,
                title,
                anchor,
                blocks,
            } => out.push(Block::Section {
                kind,
                title,
                anchor,
                blocks: hoist(blocks),
            }),
            Block::Table {
                caption: Some(c),
                header_rows,
                role,
                rows,
                alignments,
                anchor,
                layout,
            } => {
                out.push(Block::Paragraph {
                    inlines: vec![rezzer_model::Inline::plain(c)],
                    role: Some("caption".into()),
                    anchor: None,
                });
                out.push(Block::Table {
                    caption: None,
                    header_rows,
                    role,
                    rows,
                    alignments,
                    anchor,
                    layout,
                });
            }
            other => out.push(other),
        }
    }
    out
}

/// The invariants of every page of a read.
pub(crate) fn check_read(out: &ReadOutcome, page: &Page) -> Vec<Broken> {
    let mut broken = Vec::new();
    for (n, sheet) in page.sheets.iter().enumerate() {
        for b in check::check_page(out, n, &sheet.built) {
            let inv = match b.0 {
                check::Inv::RightPair => Inv::RightPair,
                check::Inv::ValueOnce => Inv::ValueOnce,
                check::Inv::ClearPairs => Inv::ClearPairs,
                check::Inv::UnclearSaid => Inv::UnclearSaid,
                check::Inv::Title => Inv::F3Title,
                check::Inv::NoteOnce => Inv::NoteOnce,
            };
            broken.push(Broken(inv, format!("page {}: {}", n + 1, b.1)));
        }
        check_want(out, n, sheet, &mut broken);
    }
    broken
}

/// The cells of a table block.
fn cells(rows: &[Vec<rezzer_model::Cell>]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|r| r.iter().map(|c| plain_text(&c.inlines)).collect())
        .collect()
}

/// The blocks of a page and the blocks inside them, depth first.
fn flat(blocks: &[Block]) -> Vec<&Block> {
    let mut out = Vec::new();
    for b in blocks {
        out.push(b);
        match b {
            Block::Section { blocks, .. }
            | Block::Quote { blocks, .. }
            | Block::Callout { blocks, .. } => out.extend(flat(blocks)),
            Block::List { items, .. } => {
                for i in items {
                    out.extend(flat(i));
                }
            }
            _ => {}
        }
    }
    out
}

/// The text of a block as [`text`] writes it, white space collapsed.
fn block_text(b: &Block) -> String {
    collapse(&text(std::slice::from_ref(b)))
}

/// Whether the words of `phrase` stand one after the other in `text`, words cut at white
/// space and cell bounds.
fn has_words(text: &str, phrase: &str) -> bool {
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || c == '|')
        .filter(|w| !w.is_empty())
        .collect();
    let want: Vec<&str> = phrase.split_whitespace().collect();
    words.windows(want.len()).any(|w| w == want.as_slice())
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The page's own checks of [`crate::axes::Want`].
fn check_want(out: &ReadOutcome, n: usize, sheet: &Sheet, broken: &mut Vec<Broken>) {
    let want = &sheet.want;
    let blocks = flat(page_blocks(out, n));
    let p = n + 1;
    for t in &want.tables {
        let found = blocks
            .iter()
            .any(|b| matches!(b, Block::Table { rows, .. } if cells(rows) == *t));
        if !found {
            let tables: Vec<Vec<Vec<String>>> = blocks
                .iter()
                .filter_map(|b| match b {
                    Block::Table { rows, .. } => Some(cells(rows)),
                    _ => None,
                })
                .collect();
            broken.push(Broken(
                Inv::Table,
                format!("page {p}: no table {t:?}; tables {tables:?}"),
            ));
        }
    }
    let headings: Vec<String> = blocks
        .iter()
        .filter_map(|b| match b {
            Block::Heading { inlines, .. } => Some(collapse(&plain_text(inlines))),
            _ => None,
        })
        .collect();
    for h in &want.headings {
        if !headings.contains(h) {
            broken.push(Broken(
                Inv::Heading,
                format!("page {p}: {h:?} is no heading; headings {headings:?}"),
            ));
        }
    }
    for h in &want.no_headings {
        if headings.iter().any(|x| x.contains(h.as_str())) {
            broken.push(Broken(
                Inv::Heading,
                format!("page {p}: {h:?} is a heading"),
            ));
        }
    }
    for (role, t) in &want.roles {
        let found = blocks.iter().any(|b| {
            matches!(b, Block::Paragraph { inlines, role: Some(r), .. }
                if r == role && collapse(&plain_text(inlines)) == *t)
        });
        if !found {
            broken.push(Broken(
                Inv::Role,
                format!("page {p}: no {role} paragraph {t:?}"),
            ));
        }
    }
    for t in &want.paragraphs {
        let found = blocks.iter().any(|b| {
            matches!(b, Block::Paragraph { inlines, .. } if collapse(&plain_text(inlines)) == *t)
        });
        if !found {
            broken.push(Broken(
                Inv::Paragraph,
                format!("page {p}: no paragraph {t:?}"),
            ));
        }
    }
    for (a, b) in &want.apart {
        if let Some(bl) = blocks
            .iter()
            .filter(|bl| !matches!(bl, Block::Section { .. }))
            .find(|bl| {
                let t = block_text(bl);
                has_words(&t, a) && has_words(&t, b)
            })
        {
            broken.push(Broken(
                Inv::Apart,
                format!(
                    "page {p}: {a:?} and {b:?} stand in one block: {:?}",
                    block_text(bl)
                ),
            ));
        }
    }
    if n == 0 {
        let props = &out.document.properties;
        if let Some(t) = &want.title {
            match &props.title {
                Some(found) if found == t => {}
                Some(found) => broken.push(Broken(
                    Inv::WrongValue,
                    format!("title {found:?}, want {t:?}"),
                )),
                None => broken.push(Broken(Inv::DocTitle, format!("no title, want {t:?}"))),
            }
        }
        if let Some(d) = &want.date {
            match &props.document_date {
                Some(found) if found.date == *d => {}
                Some(found) => broken.push(Broken(
                    Inv::WrongValue,
                    format!("document date {:?}, want {d:?}", found.date),
                )),
                None => broken.push(Broken(
                    Inv::DocDate,
                    format!("no document date, want {d:?}"),
                )),
            }
        }
        if !want.codes.is_empty() {
            let found: Vec<(String, String)> = out
                .document
                .assets
                .iter()
                .flat_map(|a| a.codes.iter())
                .map(|c| (c.symbology.clone(), c.text.clone().unwrap_or_default()))
                .collect();
            for c in &want.codes {
                let k = found.iter().filter(|f| *f == c).count();
                if k != 1 {
                    broken.push(Broken(
                        Inv::Code,
                        format!("{c:?} found {k} times: {found:?}"),
                    ));
                }
            }
            for f in &found {
                if !want.codes.contains(f) {
                    broken.push(Broken(
                        Inv::WrongValue,
                        format!("a code {f:?} not on the page"),
                    ));
                }
            }
        }
        if let Some(v) = want.vision {
            let need = needing(out);
            if need.is_empty() == v {
                broken.push(Broken(
                    Inv::Vision,
                    format!("needs vision {need:?}, want {v}"),
                ));
            }
        }
    }
    let loc = format!("p{p}");
    for l in &want.losses {
        let found =
            out.document.losses.iter().any(|x| {
                x.what.contains(l.as_str()) && (x.location == loc || x.location.is_empty())
            });
        if !found {
            broken.push(Broken(
                Inv::LossSaid,
                format!("page {p}: no loss {l:?}: {:?}", out.document.losses),
            ));
        }
    }
}

/// The reading of page `n` to compare across scales and fonts: each block's kind and its
/// text with white space collapsed, and the losses of the page by reason, severity and text
/// with every number written as `#`.
fn reading(out: &ReadOutcome, n: usize) -> (Vec<String>, Vec<String>) {
    let blocks: Vec<String> = page_blocks(out, n)
        .iter()
        .map(|b| {
            let kind = match b {
                Block::Heading { level, .. } => format!("h{level}"),
                Block::Paragraph { role, .. } => format!("p{}", role.as_deref().unwrap_or("")),
                Block::Table { header_rows, .. } => format!("t{header_rows}"),
                Block::List { .. } => "l".into(),
                Block::Media { .. } => "m".into(),
                _ => "o".into(),
            };
            format!("{kind}: {}", block_text(b))
        })
        .collect();
    (blocks, losses_of(out, n))
}

/// The losses of page `n` (and of the file), numbers written as `#`.
fn losses_of(out: &ReadOutcome, n: usize) -> Vec<String> {
    let loc = format!("p{}", n + 1);
    let mut losses: Vec<String> = out
        .document
        .losses
        .iter()
        .filter(|l| l.location == loc || !l.location.starts_with('p'))
        .map(|l| {
            let mut what = String::new();
            let mut digit = false;
            for c in l.what.chars() {
                if c.is_ascii_digit() || (digit && (c == '.' || c == ',')) {
                    if !digit {
                        what.push('#');
                    }
                    digit = true;
                } else {
                    digit = false;
                    what.push(c);
                }
            }
            format!("{:?} {:?}: {what}", l.why, l.severity)
        })
        .collect();
    losses.sort();
    losses
}

/// What a variant is held to, as text: its fields, values, columns and wants.
fn fingerprint(page: &Page) -> String {
    page.sheets
        .iter()
        .map(|s| {
            format!(
                "{:?} {:?} {:?} {:?}",
                s.built.fields, s.built.values, s.built.columns, s.want
            )
        })
        .collect()
}

/// The text of every page of a read, block by block.
fn out_text(out: &ReadOutcome) -> Vec<Vec<String>> {
    (0..out.document.body.len())
        .map(|n| reading(out, n).0)
        .filter(|p| !p.is_empty())
        .collect()
}

/// The variants of `shape` read and held to the invariants, each with what it broke: the
/// invariants of its own page, and against the read of its reference (the same variant at
/// scale 1 in the real sizes and fonts), when both are held to the same things and the
/// reference holds its own invariants, the same reading, and no loss when the reference has
/// none.
pub(crate) fn reads(shape: &Shape) -> Vec<(String, Vec<Broken>)> {
    let mut done: HashMap<String, (ReadOutcome, String, bool)> = HashMap::new();
    let mut out = Vec::new();
    let sample: Vec<Page> = variants(shape).collect();
    for page in &sample {
        let read_out = read(page);
        // `VARIANT_DUMP="shape|variant"` prints that read, to look at a break.
        if std::env::var("VARIANT_DUMP").ok() == Some(format!("{}|{}", shape.name, page.name)) {
            eprintln!(
                "{:#?}\n{:#?}\n{:#?}",
                out_text(&read_out),
                read_out.document.losses,
                read_out.document.properties
            );
        }
        let mut broken = check_read(&read_out, page);
        let r = page.variant.reference();
        if r != page.variant {
            let key = r.name();
            if !done.contains_key(&key) {
                let owned;
                let rp = match sample.iter().find(|p| p.variant == r) {
                    Some(p) => p,
                    None => {
                        owned = page_of(shape, r);
                        &owned
                    }
                };
                let ref_out = read(rp);
                let holds = check_read(&ref_out, rp).is_empty();
                done.insert(key.clone(), (ref_out, fingerprint(rp), holds));
            }
            let (ref_out, ref_print, holds) = &done[&key];
            // A reference that breaks its own invariants is no reading to hold others to:
            // its own break is pinned.
            let pages = if *holds { page.sheets.len() } else { 0 };
            for n in 0..pages {
                let (mine, theirs) = (reading(&read_out, n), reading(ref_out, n));
                if *ref_print == fingerprint(page) && mine.0 != theirs.0 {
                    let only = |a: &[String], b: &[String]| -> Vec<String> {
                        a.iter().filter(|x| !b.contains(x)).cloned().collect()
                    };
                    let (here, there) = (only(&mine.0, &theirs.0), only(&theirs.0, &mine.0));
                    let says = if here.is_empty() && there.is_empty() {
                        format!("in another order: {:?}; at scale 1 {:?}", mine.0, theirs.0)
                    } else {
                        format!("here {here:?}; at scale 1 {there:?}")
                    };
                    broken.push(Broken(Inv::SameReading, format!("page {}: {says}", n + 1)));
                }
                if *ref_print == fingerprint(page) && theirs.1.is_empty() && !mine.1.is_empty() {
                    broken.push(Broken(Inv::NoLoss, format!("page {}: {:?}", n + 1, mine.1)));
                }
            }
        }
        out.push((page.name.clone(), broken));
    }
    out
}

/// The variants of `shape` that do not hold as [`KNOWN`] pins them: a variant that is not
/// known and breaks an invariant, a known one that breaks other invariants than its entry
/// names or holds them all, and a known entry no variant of the sample has.
///
/// With `VARIANT_UNPINNED` set, no entry pins a break: every break is listed, to write the
/// entries anew after a fix.
pub(crate) fn unexpected(shape: &Shape, reads: &[(String, Vec<Broken>)]) -> Vec<String> {
    let known = if std::env::var("VARIANT_UNPINNED").is_ok() {
        &[]
    } else {
        KNOWN
    };
    unexpected_in(shape.name, reads, known)
}

/// [`unexpected`] against the entries `known`.
pub(crate) fn unexpected_in(
    shape: &str,
    reads: &[(String, Vec<Broken>)],
    known: &[Known],
) -> Vec<String> {
    let mine: Vec<&Known> = known.iter().filter(|k| k.shape == shape).collect();
    let mut out = Vec::new();
    for (name, broken) in reads {
        let set: BTreeSet<Inv> = broken.iter().map(|b| b.0).collect();
        let entry = mine.iter().find(|k| k.variants.contains(&name.as_str()));
        let held = match entry {
            Some(k) => set == k.broken.iter().copied().collect::<BTreeSet<Inv>>(),
            None => set.is_empty(),
        };
        if !held {
            let list: Vec<String> = broken
                .iter()
                .map(|b| format!("{:?}: {}", b.0, b.1))
                .collect();
            out.push(format!(
                "{shape} | {name} | {set:?} | known as {:?}\n  {}",
                entry.map(|k| (k.broken, k.cause)),
                list.join("\n  ")
            ));
        }
    }
    for k in mine {
        for v in k.variants {
            if !reads.iter().any(|(n, _)| n == v) {
                out.push(format!("{shape} | {v} | known, but not in the sample"));
            }
        }
    }
    out
}

/// Reads every variant of `shape` and asserts that each holds as pinned.
pub(crate) fn hold(shape: &Shape) {
    let reads = reads(shape);
    let wrong = unexpected(shape, &reads);
    if std::env::var("VARIANT_COUNT").is_ok() {
        eprintln!("VARIANTS {} {}", reads.len(), shape.name);
    }
    assert!(
        wrong.is_empty(),
        "{} of {} variants of {} do not hold as pinned:\n{}",
        wrong.len(),
        reads.len(),
        shape.name,
        wrong.join("\n")
    );
}
