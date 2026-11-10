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
    /// mixes them is a wrong reading order); in a block kept as laid out, not onto one line.
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
    /// A page that draws no code, or whose codes are all found as often as they are drawn,
    /// says of nothing on it that it looks like a code (no false loss on rules, ticks,
    /// hatching or a code read). The words of a read that is not listed ("a part read as
    /// EAN-13 ..., not listed") are allowed: they list no value, and on a screen or letters
    /// they are loss noise only (cosmetic); a listed value there breaks WrongValue.
    NoCode,
    /// The page needs vision exactly when it should.
    Vision,
    /// Each expected loss is said.
    LossSaid,
    /// The read equals the read of the same variant at scale 1 in the real sizes and fonts.
    SameReading,
    /// A variant whose read at scale 1 in the real sizes and fonts has no loss has none.
    NoLoss,
    /// Every run of a page is in exactly one region (a run in none only on a page whose
    /// layout stopped, which its limit loss says).
    Cover,
    /// Every region's kind is the kind of the block it stands at (its host) and of the blocks
    /// its runs with ink were written in (figures and codes aside: their media block comes
    /// later).
    KindWritten,
    /// Each text of [`crate::axes::Want::kinds`] stands in a region of its kind, and each text
    /// of [`crate::axes::Want::spatial`] in a spatial region (Task 20).
    RegionKind,
    /// A text the shape marks unclear (a field whose layout leaves it unclear) is never in a
    /// `table` or `label_value` region (P6-X: a type forced on it reads a wrong pair or cell).
    NoForcedType,
    /// A spatial region is written as a block kept as laid out, that block always has its loss,
    /// and the texts of its runs stand in it once each and in their order (P6-X: a block kept
    /// as laid out with no word of why, or with a text lost, doubled or moved).
    SpatialSaid,
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
                | Inv::NoForcedType
                | Inv::SpatialSaid
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
    read_engine(page).0
}

/// [`read`] with each page as the region engine laid it out.
pub(crate) fn read_engine(page: &Page) -> (ReadOutcome, Vec<rezzer_read_pdf::EnginePage>) {
    let (mut out, pages) = crate::common::read_engine_pages(&file(page));
    out.document.body = hoist(std::mem::take(&mut out.document.body));
    (out, pages)
}

/// The kind of a region whose runs were written in a block of the word `word` (see
/// [`rezzer_read_pdf::block_word`]), if it says one; whether a region of `kind` agrees with
/// it.
fn agrees(kind: layoutregions::Kind, word: &str) -> Option<bool> {
    use layoutregions::Kind as K;
    let ok: &[&str] = match kind {
        K::Figure | K::Code => return None,
        K::Heading | K::TitleBlock => &["heading"],
        K::Paragraph => &["paragraph"],
        K::List => &["list"],
        K::Table | K::LabelValue | K::Totals => &["table"],
        K::Spatial => &["spatial"],
        K::HeaderBand => &["page-header"],
        K::FooterBand => &["page-footer"],
        K::FootnoteLegend => &["section", "footnote"],
    };
    Some(ok.contains(&word))
}

/// [`Inv::Cover`] and [`Inv::KindWritten`] over the pages of a read.
pub(crate) fn check_engine(pages: &[rezzer_read_pdf::EnginePage]) -> Vec<Broken> {
    let mut broken = Vec::new();
    for p in pages {
        let n = p.regions.page;
        let mut count = vec![0u32; p.input.texts.len()];
        for r in &p.regions.regions {
            let mut wrong: Vec<(&str, &str)> = Vec::new();
            for id in &r.prims {
                let layoutregions::PrimId::Text(i) = *id else {
                    continue;
                };
                if let Some(c) = count.get_mut(i as usize) {
                    *c += 1;
                }
                let text = p.input.texts[i as usize].text.as_str();
                if text.trim().is_empty() {
                    continue;
                }
                if let Some(word) = p.run_blocks.get(i as usize).copied().flatten()
                    && agrees(r.kind, word) == Some(false)
                {
                    wrong.push((word, text));
                }
            }
            if !wrong.is_empty() {
                broken.push(Broken(
                    Inv::KindWritten,
                    format!("page {n}: a {} region written as {wrong:?}", r.kind.name()),
                ));
            }
            if let Some(word) = p.block_words.get(r.host as usize)
                && agrees(r.kind, word) == Some(false)
            {
                broken.push(Broken(
                    Inv::KindWritten,
                    format!(
                        "page {n}: a {} region {:?} stands at a {word} block",
                        r.kind.name(),
                        r.signals.iter().map(|s| s.id).collect::<Vec<_>>()
                    ),
                ));
            }
        }
        let none: Vec<usize> = (0..count.len()).filter(|&i| count[i] == 0).collect();
        let twice: Vec<usize> = (0..count.len()).filter(|&i| count[i] > 1).collect();
        if (!none.is_empty() && !p.stopped) || !twice.is_empty() {
            broken.push(Broken(
                Inv::Cover,
                format!("page {n}: runs in no region {none:?}, in two {twice:?}"),
            ));
        }
    }
    broken
}

/// A region as the region checks see it: its kind's name, the place of the block it stands
/// at, and the texts of its runs in their order on the page (by line, then left to right).
#[derive(Debug, Clone)]
pub(crate) struct Seen {
    pub kind: &'static str,
    pub host: u32,
    pub texts: Vec<String>,
}

impl Seen {
    /// The texts of the region as one text.
    fn joined(&self) -> String {
        self.texts.join(" ")
    }
}

/// The regions of an engine page, as [`Seen`].
fn seen(p: &rezzer_read_pdf::EnginePage) -> Vec<Seen> {
    p.regions
        .regions
        .iter()
        .map(|r| {
            let mut runs: Vec<&layoutregions::TextRun> = r
                .prims
                .iter()
                .filter_map(|id| match *id {
                    layoutregions::PrimId::Text(i) => p.input.texts.get(i as usize),
                    _ => None,
                })
                .filter(|t| !t.text.trim().is_empty())
                .collect();
            runs.sort_by(|a, b| a.line.cmp(&b.line).then(a.x.total_cmp(&b.x)));
            Seen {
                kind: r.kind.name(),
                host: r.host,
                texts: runs.iter().map(|t| t.text.clone()).collect(),
            }
        })
        .collect()
}

/// The words of a text for [`holds`]: cut at white space and cell bounds, a colon at a word's
/// end left off.
fn words(t: &str) -> Vec<&str> {
    t.split(|c: char| c.is_whitespace() || c == '|')
        .map(|w| w.trim_end_matches(':'))
        .filter(|w| !w.is_empty())
        .collect()
}

/// Whether the words of `phrase` stand one after the other in `text`.
fn holds(text: &str, phrase: &str) -> bool {
    let (text, want) = (words(text), words(phrase));
    !want.is_empty() && text.windows(want.len()).any(|w| w == want.as_slice())
}

/// [`Inv::NoForcedType`] on page `p` (from 1): each text of `unclear` that stands in a `table`
/// or `label_value` region of `regions` breaks it.
pub(crate) fn forced(unclear: &[&str], regions: &[Seen], p: usize) -> Vec<Broken> {
    let mut broken = Vec::new();
    for t in unclear {
        for r in regions {
            if matches!(r.kind, "table" | "label_value") && holds(&r.joined(), t) {
                broken.push(Broken(
                    Inv::NoForcedType,
                    format!(
                        "page {p}: the unclear {t:?} stands in a {} region {:?}",
                        r.kind, r.texts
                    ),
                ));
            }
        }
    }
    broken
}

/// The text of a block kept as laid out: its pieces line by line, left to right.
fn spatial_text(lines: &[rezzer_model::SpatialLine]) -> String {
    lines
        .iter()
        .flat_map(|l| &l.pieces)
        .map(|p| p.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// [`Inv::RegionKind`], [`Inv::NoForcedType`] and [`Inv::SpatialSaid`] over the pages of a
/// read and the engine's layout of them.
pub(crate) fn check_regions(
    out: &ReadOutcome,
    engine: &[rezzer_read_pdf::EnginePage],
    page: &Page,
) -> Vec<Broken> {
    let mut broken = Vec::new();
    for (n, sheet) in page.sheets.iter().enumerate() {
        let p = n + 1;
        let Some(e) = engine.iter().find(|e| e.regions.page as usize == p) else {
            continue;
        };
        let regions = seen(e);
        let want = &sheet.want;
        let wanted = want
            .kinds
            .iter()
            .map(|(t, k)| (t, *k))
            .chain(want.spatial.iter().map(|t| (t, "spatial")));
        for (t, kind) in wanted {
            let at: Vec<&str> = regions
                .iter()
                .filter(|r| holds(&r.joined(), t))
                .map(|r| r.kind)
                .collect();
            if !at.contains(&kind) {
                broken.push(Broken(
                    Inv::RegionKind,
                    format!("page {p}: {t:?} stands in no {kind} region; in {at:?}"),
                ));
            }
        }
        let unclear: Vec<&str> = sheet
            .built
            .fields
            .iter()
            .filter(|f| f.1.is_err())
            .map(|f| f.0)
            .collect();
        broken.extend(forced(&unclear, &regions, p));
        broken.extend(spatial_said(out, n, e, &regions));
    }
    broken
}

/// [`Inv::SpatialSaid`] on page `n` (from 0): every block kept as laid out has its loss, and
/// the spatial regions that stand at one are written in it, every text of their runs once
/// and in its order.
fn spatial_said(
    out: &ReadOutcome,
    n: usize,
    e: &rezzer_read_pdf::EnginePage,
    regions: &[Seen],
) -> Vec<Broken> {
    let p = n + 1;
    let mut broken = Vec::new();
    let said = |at: &str| {
        out.document
            .losses
            .iter()
            .any(|l| l.location == at && l.what.contains("kept as laid out"))
    };
    let blocks = flat(page_blocks(out, n));
    for b in &blocks {
        if let Block::Spatial { lines, anchor, .. } = b {
            let at = anchor.as_ref().map(|a| a.0.as_str()).unwrap_or_default();
            if !said(at) {
                broken.push(Broken(
                    Inv::SpatialSaid,
                    format!(
                        "page {p}: the block at {at:?} is kept as laid out with no loss: {:?}",
                        spatial_text(lines)
                    ),
                ));
            }
        }
    }
    let mut hosts: Vec<u32> = regions
        .iter()
        .filter(|r| r.kind == "spatial")
        .map(|r| r.host)
        .collect();
    hosts.sort_unstable();
    hosts.dedup();
    for host in hosts {
        let at = e.anchors.get(host as usize).cloned().flatten();
        let block = at.as_deref().and_then(|at| {
            blocks.iter().find_map(|b| match b {
                Block::Spatial { lines, anchor, .. }
                    if anchor.as_ref().is_some_and(|a| a.0 == at) =>
                {
                    Some(spatial_text(lines))
                }
                _ => None,
            })
        });
        let texts: Vec<&str> = regions
            .iter()
            .filter(|r| r.kind == "spatial" && r.host == host)
            .flat_map(|r| r.texts.iter().map(String::as_str))
            .collect();
        let Some(block) = block else {
            broken.push(Broken(
                Inv::SpatialSaid,
                format!("page {p}: a spatial region {texts:?} stands at no spatial block ({at:?})"),
            ));
            continue;
        };
        let bare = |t: &str| t.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        if bare(&block) != bare(&texts.concat()) {
            broken.push(Broken(
                Inv::SpatialSaid,
                format!(
                    "page {p}: the spatial block at {} holds {block:?}, its runs read {texts:?}",
                    at.unwrap_or_default()
                ),
            ));
        }
    }
    broken
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

/// The lines of a heading or paragraph, parted at its line breaks, white space collapsed; a
/// table row is one line; other blocks have none here.
fn lines_of(b: &Block) -> Vec<String> {
    match b {
        Block::Heading { inlines, .. } | Block::Paragraph { inlines, .. } => inlines
            .split(|i| matches!(i, rezzer_model::Inline::LineBreak))
            .map(|part| collapse(&plain_text(part)))
            .collect(),
        Block::Table { rows, .. } => cells(rows).iter().map(|r| r.join(" | ")).collect(),
        _ => Vec::new(),
    }
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
    // A block kept as laid out holds each text where it stands: two texts on two of its
    // lines, each in its column, are not read into one (ruling on 19a-4), so it is checked
    // line by line; any other block as a whole.
    for (a, b) in &want.apart {
        if let Some(t) = blocks
            .iter()
            .filter(|bl| !matches!(bl, Block::Section { .. }))
            .flat_map(|bl| match bl {
                Block::Spatial { lines, .. } => lines
                    .iter()
                    .map(|l| {
                        let pieces: Vec<&str> = l.pieces.iter().map(|p| p.text.as_str()).collect();
                        collapse(&pieces.join(" "))
                    })
                    .collect(),
                _ => vec![block_text(bl)],
            })
            .find(|t| has_words(t, a) && has_words(t, b))
        {
            broken.push(Broken(
                Inv::Apart,
                format!("page {p}: {a:?} and {b:?} stand in one block: {t:?}"),
            ));
        }
    }
    for (a, b) in &want.lines_apart {
        if let Some(l) = blocks
            .iter()
            .flat_map(|bl| lines_of(bl))
            .find(|l| has_words(l, a) && has_words(l, b))
        {
            broken.push(Broken(
                Inv::Cells,
                format!("page {p}: {a:?} and {b:?} stand on one line: {l:?}"),
            ));
        }
    }
    // A line of a block kept as laid out is a line of the read too: the engine keeps a right
    // read of one short line as laid out where no region type has clear evidence for it
    // (Task 19, cosmetic; Task 36 makes it a paragraph again), and its words still go on in
    // one line.
    for (a, b) in &want.lines_joined {
        let one = blocks
            .iter()
            .flat_map(|bl| match bl {
                Block::Spatial { lines, .. } => lines
                    .iter()
                    .map(|l| {
                        let pieces: Vec<&str> = l.pieces.iter().map(|p| p.text.as_str()).collect();
                        collapse(&pieces.join(" "))
                    })
                    .collect(),
                _ => lines_of(bl),
            })
            .any(|l| has_words(&l, &format!("{a} {b}")));
        if !one {
            broken.push(Broken(
                Inv::Paragraph,
                format!("page {p}: {a:?} and {b:?} do not go on in one line"),
            ));
        }
    }
    check_cells(out, n, sheet, broken);
    if !want.glyph_text.is_empty() {
        let text: Vec<String> = blocks.iter().map(|b| block_text(b)).collect();
        for g in &want.glyph_text {
            let odd: Vec<char> = g.chars().filter(|c| !c.is_ascii()).collect();
            if let Some(t) = text
                .iter()
                .find(|t| t.contains(g.as_str()) || odd.iter().any(|c| t.contains(*c)))
            {
                broken.push(Broken(
                    Inv::WrongValue,
                    format!("page {p}: the glyph text {g:?} of a code stands in the read: {t:?}"),
                ));
            }
        }
    }
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
    // Words that a code is there, other than a read not listed.
    let doubt = assets.iter().any(|a| {
        let alt = a.alt.as_deref().unwrap_or_default();
        a.code_unread
            && [
                "like a barcode",
                "not decoded",
                "not scanned",
                "lighter line",
                "dark line",
            ]
            .iter()
            .any(|w| alt.contains(w))
    });
    let mut checked: Vec<&(String, String)> = Vec::new();
    let mut all_found = true;
    for c in &want.codes {
        if checked.contains(&c) {
            continue;
        }
        checked.push(c);
        let drawn = want.codes.iter().filter(|w| *w == c).count();
        let k = found.iter().filter(|f| *f == c).count();
        all_found &= k == drawn;
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
    if all_found && doubt {
        broken.push(Broken(
            Inv::NoCode,
            format!(
                "page {p}: every code drawn is found ({} drawn), but a picture says a code \
                 there is not decoded: {:?}",
                want.codes.len(),
                assets
                    .iter()
                    .filter(|a| a.code_unread)
                    .map(|a| a.alt.as_deref().unwrap_or_default())
                    .collect::<Vec<_>>()
            ),
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
        let (read_out, engine) = read_engine(page);
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
        broken.extend(check_engine(&engine));
        broken.extend(check_regions(&read_out, &engine, page));
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
