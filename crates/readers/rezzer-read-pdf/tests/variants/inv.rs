// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The invariants every variant is held to, and the run of a shape's variants against its
//! pinned known breaks.

use std::collections::{BTreeSet, HashMap};

use rezzer_model::{Block, ReadOutcome, plain_text};

use crate::axes::{Page, Shape, Sheet, Variant, page_of, variants};
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
    /// Texts that belong to different blocks are not read into one block (P6-X: a block that
    /// mixes them is a wrong reading order).
    Apart,
    /// The texts of each drawn cell stand together (and, for a cell the shape lists, in their
    /// order), and no block or table row holds texts of two cells, found by the span each
    /// drawn text produced (P6-X: a wrong reading order).
    Cells,
    /// The page's text holds the expected texts in their reading order.
    Order,
    /// The document title is found.
    DocTitle,
    /// The document date is found.
    DocDate,
    /// Each code is found as many times as it is drawn.
    Code,
    /// A code is never found more often than it is drawn, and a page with a code not found
    /// says that a code there is not decoded (P6-X: never a value doubled or lost silently).
    CodeSaid,
    /// A code found with a box is boxed by the bars of a code drawn: within a point at scale
    /// 1, more on a larger page.
    CodeBox,
    /// A page that draws no code says of nothing on it that it looks like a code (no false
    /// loss on rules, ticks and hatching).
    NoCode,
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
    /// The invariants of ruling P6-X: a break is a wrong value or a wrong reading order,
    /// never only a lost one.
    pub(crate) fn wrong(self) -> bool {
        matches!(
            self,
            Inv::RightPair
                | Inv::ValueOnce
                | Inv::WrongValue
                | Inv::Apart
                | Inv::Cells
                | Inv::CodeSaid
        )
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
    // A code of the file that no page draws is a wrong value, shown on a page or not.
    let drawn: Vec<&(String, String)> = page.sheets.iter().flat_map(|s| &s.want.codes).collect();
    for a in &out.document.assets {
        for c in &a.codes {
            let c = (c.symbology.clone(), c.text.clone().unwrap_or_default());
            if !drawn.contains(&&c) {
                broken.push(Broken(
                    Inv::WrongValue,
                    format!("a code {c:?} of {} not drawn on any page", a.id),
                ));
            }
        }
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

/// The cells of a page held to [`Inv::Cells`]: the cells the shape lists, in their order, and
/// each field's own cell (its label and its value) that no listed cell holds, in any order (a
/// label under its value reads after it).
fn check_cells(out: &ReadOutcome, n: usize, sheet: &Sheet, broken: &mut Vec<Broken>) {
    let mut cells: Vec<Vec<String>> = sheet.want.cells.clone();
    let mut ordered = vec![true; cells.len()];
    for (label, value) in &sheet.built.fields {
        if cells.iter().any(|c| c.iter().any(|t| t == label)) {
            continue;
        }
        let mut cell = vec![(*label).to_string()];
        if let Ok(Some(v)) = value {
            cell.push((*v).to_string());
        }
        cells.push(cell);
        ordered.push(false);
    }
    if cells.is_empty() {
        return;
    }
    let drawn: Vec<String> = crate::axes::drawn(&sheet.built.page)
        .iter()
        .map(|t| collapse(t))
        .collect();
    // A text drawn more than once, or as a part of more than one drawn text, is not checked.
    let ambiguous = |t: &str| {
        let t = collapse(t);
        drawn
            .iter()
            .map(|d| d.matches(t.as_str()).count())
            .sum::<usize>()
            != 1
    };
    let units = crate::common::read_units(page_blocks(out, n));
    let found = crate::common::cells_check(&units, &cells, &ordered, &ambiguous);
    for w in &found.wrong {
        broken.push(Broken(Inv::Cells, format!("page {}: {w}", n + 1)));
    }
    // The cells a shape lists are checked: a list whose every text is left out (drawn twice or
    // inside other text) checks nothing, so it fails.
    let listed: Vec<bool> = (0..cells.len())
        .map(|c| c < sheet.want.cells.len())
        .collect();
    if found.all_skipped(&cells, &listed) {
        broken.push(Broken(
            Inv::Cells,
            format!(
                "page {}: every text of the listed cells is left out of the check ({} texts): \
                 each is drawn more than once or inside other text",
                n + 1,
                found.skipped.iter().sum::<usize>()
            ),
        ));
    }
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
    if !want.order.is_empty() {
        let page_text = collapse(&text(page_blocks(out, n)));
        let order: Vec<&str> = want.order.iter().map(String::as_str).collect();
        if let Err(e) = crate::common::in_order(&page_text, &order) {
            broken.push(Broken(
                Inv::Order,
                format!("page {p}: {e:?} is not in its place: {page_text:?}"),
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
    check_cells(out, n, sheet, broken);
    if n == 0 {
        let props = &out.document.properties;
        // A found title is the page's title, or a title the page may have besides (a bold
        // title over a block); a page that names none may have no other found.
        match (&want.title, &props.title) {
            (Some(t), Some(found)) if found == t => {}
            (_, Some(found)) if want.may_title.contains(found) => {}
            (want_title, Some(found)) => broken.push(Broken(
                Inv::WrongValue,
                format!("title {found:?}, want {want_title:?}"),
            )),
            (Some(t), None) => broken.push(Broken(Inv::DocTitle, format!("no title, want {t:?}"))),
            (None, None) => {}
        }
        // A page that names no document date may have none found: any found date is wrong.
        match (&want.date, &props.document_date) {
            (Some(d), Some(found)) if found.date == *d => {}
            (want, Some(found)) => broken.push(Broken(
                Inv::WrongValue,
                format!("document date {:?}, want {want:?}", found.date),
            )),
            (Some(d), None) => broken.push(Broken(
                Inv::DocDate,
                format!("no document date, want {d:?}"),
            )),
            (None, None) => {}
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
    // The codes of this page: those of the assets its picture blocks show, once for each
    // block (a code drawn the same on every page, or twice on one, is one asset, shown at
    // each place).
    let shown: Vec<&str> = blocks
        .iter()
        .filter_map(|b| match b {
            Block::Media { asset, .. } => Some(asset.as_str()),
            _ => None,
        })
        .collect();
    let assets: Vec<&rezzer_model::Asset> = shown
        .iter()
        .filter_map(|id| out.document.assets.iter().find(|a| a.id == *id))
        .collect();
    let found: Vec<(String, String)> = assets
        .iter()
        .flat_map(|a| a.codes.iter())
        .map(|c| (c.symbology.clone(), c.text.clone().unwrap_or_default()))
        .collect();
    let unread = assets.iter().any(|a| a.code_unread);
    let mut checked: Vec<&(String, String)> = Vec::new();
    for c in &want.codes {
        if checked.contains(&c) {
            continue;
        }
        checked.push(c);
        let drawn = want.codes.iter().filter(|w| *w == c).count();
        let k = found.iter().filter(|f| *f == c).count();
        if k != drawn {
            broken.push(Broken(
                Inv::Code,
                format!("page {p}: {c:?} found {k} times, drawn {drawn}: {found:?}"),
            ));
        }
        if k > drawn || (k < drawn && !unread) {
            broken.push(Broken(
                Inv::CodeSaid,
                format!(
                    "page {p}: {c:?} found {k} times, drawn {drawn}, a code not decoded said: \
                     {unread}"
                ),
            ));
        }
    }
    if want.codes.is_empty() && unread {
        broken.push(Broken(
            Inv::NoCode,
            format!("page {p}: no code is drawn, but a picture says a code there is not decoded"),
        ));
    }
    let scale = sheet.built.page.scale;
    for a in &assets {
        for c in &a.codes {
            let Some(b) = c.bounds else { continue };
            let near = |bars: &[f32; 4]| {
                b.iter()
                    .zip(bars)
                    .all(|(got, want)| (got - want * scale).abs() <= scale.max(1.0))
            };
            if !want.code_bars.is_empty() && !want.code_bars.iter().any(near) {
                broken.push(Broken(
                    Inv::CodeBox,
                    format!(
                        "page {p}: a code boxed {b:?}, the bars are {:?} at scale {scale}",
                        want.code_bars
                    ),
                ));
            }
        }
    }
    for f in &found {
        if !want.codes.contains(f) {
            broken.push(Broken(
                Inv::WrongValue,
                format!("page {p}: a code {f:?} not on the page"),
            ));
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
    for l in &want.no_losses {
        if let Some(x) =
            out.document.losses.iter().find(|x| {
                x.what.contains(l.as_str()) && (x.location == loc || x.location.is_empty())
            })
        {
            broken.push(Broken(
                Inv::NoLoss,
                format!("page {p}: a loss {l:?} that is noise: {:?}", x.what),
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

/// A reference read: the read, what its variant is held to, and whether it breaks nothing
/// or, when it breaks something, only invariants of ruling P6-X.
struct Reference {
    out: ReadOutcome,
    print: String,
    holds: bool,
    wrong_only: bool,
}

/// Reads the variant `v` of `shape` as a reference (from the sample when it is in it).
fn reference(shape: &Shape, sample: &[Page], v: Variant) -> Reference {
    let owned;
    let rp = match sample.iter().find(|p| p.variant == v) {
        Some(p) => p,
        None => {
            owned = page_of(shape, v);
            &owned
        }
    };
    let out = read(rp);
    let broken = check_read(&out, rp);
    Reference {
        print: fingerprint(rp),
        holds: broken.is_empty(),
        wrong_only: !broken.is_empty() && broken.iter().all(|b| b.0.wrong()),
        out,
    }
}

/// The key of the reference read of `v` in `done`, read now when it is not there yet.
fn taken(
    done: &mut HashMap<String, Reference>,
    shape: &Shape,
    sample: &[Page],
    v: Variant,
) -> String {
    let key = v.name();
    if !done.contains_key(&key) {
        done.insert(key.clone(), reference(shape, sample, v));
    }
    key
}

/// What the reads of a shape gave: each variant with what it broke, and how many variants
/// were not compared with a reference because it (and its fallback) breaks.
pub(crate) struct Reads {
    pub reads: Vec<(String, Vec<Broken>)>,
    pub uncompared: usize,
    /// Variants held to other things than their reference (a font that makes a field
    /// unclear), so not compared.
    pub other: usize,
}

/// The variants of `shape` read and held to the invariants, each with what it broke: the
/// invariants of its own page, and against the read of its reference (the same variant at
/// scale 1 in the real sizes and fonts), when both are held to the same things and the
/// reference holds its own invariants, the same reading (blocks and losses), and no loss when
/// the reference has none. When the reference breaks only invariants of ruling P6-X (pinned),
/// the variant is compared with the fallback reference instead: the reference in the real look
/// and not turned, which must read the same (ruling P6-Y), when that one holds. Else the
/// variant is not compared, and counted.
pub(crate) fn reads(shape: &Shape) -> Reads {
    let mut done: HashMap<String, Reference> = HashMap::new();
    let mut out = Vec::new();
    let (mut uncompared, mut other) = (0, 0);
    let sample: Vec<Page> = variants(shape).collect();
    for page in &sample {
        let read_out = read(page);
        // `VARIANT_DUMP="shape|variant"` prints that read, to look at a break.
        if std::env::var("VARIANT_DUMP").ok() == Some(format!("{}|{}", shape.name, page.name)) {
            eprintln!(
                "{:#?}\n{:#?}\n{:#?}\n{:#?}",
                out_text(&read_out),
                read_out.document.losses,
                read_out.document.properties,
                read_out
                    .document
                    .assets
                    .iter()
                    .map(|a| (&a.id, &a.codes))
                    .collect::<Vec<_>>()
            );
        }
        let mut broken = check_read(&read_out, page);
        let r = page.variant.reference();
        if r != page.variant {
            let mut key = taken(&mut done, shape, &sample, r);
            if !done[&key].holds && done[&key].wrong_only {
                let fallback = Variant {
                    look: crate::axes::Look::Real,
                    turn: crate::axes::Turn::None,
                    ..r
                };
                if fallback != page.variant && shape.axes.look.contains(&fallback.look) {
                    key = taken(&mut done, shape, &sample, fallback);
                }
            }
            let rf = &done[&key];
            let mine_print = fingerprint(page);
            if !rf.holds {
                uncompared += 1;
            } else if rf.print != mine_print {
                other += 1;
            } else {
                for n in 0..page.sheets.len() {
                    compare(&read_out, &rf.out, n, &mut broken);
                }
            }
        }
        out.push((page.name.clone(), broken));
    }
    Reads {
        reads: out,
        uncompared,
        other,
    }
}

/// Holds page `n` of `mine` to the reading of `theirs`: the same blocks, the same losses (by
/// kind and the fields they name, numbers masked), and no loss where `theirs` has none.
pub(crate) fn compare(
    mine: &ReadOutcome,
    theirs: &ReadOutcome,
    n: usize,
    broken: &mut Vec<Broken>,
) {
    let (mine, theirs) = (reading(mine, n), reading(theirs, n));
    let only = |a: &[String], b: &[String]| -> Vec<String> {
        a.iter().filter(|x| !b.contains(x)).cloned().collect()
    };
    if mine.0 != theirs.0 {
        let (here, there) = (only(&mine.0, &theirs.0), only(&theirs.0, &mine.0));
        let says = if here.is_empty() && there.is_empty() {
            format!("in another order: {:?}; at scale 1 {:?}", mine.0, theirs.0)
        } else {
            format!("here {here:?}; at scale 1 {there:?}")
        };
        broken.push(Broken(Inv::SameReading, format!("page {}: {says}", n + 1)));
    }
    if theirs.1.is_empty() {
        if !mine.1.is_empty() {
            broken.push(Broken(Inv::NoLoss, format!("page {}: {:?}", n + 1, mine.1)));
        }
    } else if mine.1 != theirs.1 {
        broken.push(Broken(
            Inv::SameReading,
            format!(
                "page {}: losses {:?}; at scale 1 {:?}",
                n + 1,
                only(&mine.1, &theirs.1),
                only(&theirs.1, &mine.1)
            ),
        ));
    }
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
    let Reads {
        reads,
        uncompared,
        other,
    } = reads(shape);
    let wrong = unexpected(shape, &reads);
    // Shown with the test's output: the variants that could not be held to a reading.
    eprintln!(
        "VARIANTS {} of {}: {} not compared as their reference breaks, {} as they are held \
         to other things",
        reads.len(),
        shape.name,
        uncompared,
        other
    );
    assert!(
        wrong.is_empty(),
        "{} of {} variants of {} do not hold as pinned:\n{}",
        wrong.len(),
        reads.len(),
        shape.name,
        wrong.join("\n")
    );
}
