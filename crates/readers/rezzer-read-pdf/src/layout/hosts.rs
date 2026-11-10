// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Where each decision of a page stands among its blocks: the block it became (its region's
//! `host`), or the block near it for a decision that became no block of its own (a picture or a
//! code, whose media block the page places later; rows of a tagged table begun on an earlier
//! page; anything a stop left unbuilt).

use std::collections::{BTreeMap, HashMap};

use layoutregions::{Bounds, Kind, PageIn, PrimId};
use rezzer_model::Block;

use super::Built;
use super::decide::{Class, Decided};

/// The `host` of a region with no block on its page.
pub(super) const NO_BLOCK: u32 = u32::MAX;

/// What a placed part (a table, a title or a block kept as laid out) became among the blocks
/// of its page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Became {
    /// The block at this place in the page's blocks.
    Block(usize),
    /// Rows of the tagged table (by its index) begun on an earlier page.
    Earlier(usize),
    /// No block: a stop came first, or it was a note only.
    Nothing,
}

/// What the placed part built from `blocks[first..]` became: its table, or else its first
/// block that is not a found-by note (a title, a caption, a block kept as laid out, or the
/// table laid out as paragraphs). Rows for a tagged table begun on an earlier page (`earlier`)
/// build no block.
pub(super) fn became(blocks: &[Block], first: usize, earlier: Option<usize>) -> Became {
    let new = blocks.get(first..).unwrap_or_default();
    let table = new.iter().position(|b| matches!(b, Block::Table { .. }));
    let other = || {
        new.iter().position(
            |b| !matches!(b, Block::Paragraph { role, .. } if role.as_deref() == Some("found-by")),
        )
    };
    match (table.or_else(other), earlier) {
        (Some(k), _) => Became::Block(first + k),
        (None, Some(t)) => Became::Earlier(t),
        (None, None) => Became::Nothing,
    }
}

/// The first of `blocks` built with the paragraph role `role`, inside a footnote section too.
fn first_with_role(blocks: &[Block], role: &str) -> Option<usize> {
    let has =
        |b: &Block| matches!(b, Block::Paragraph { role: r, .. } if r.as_deref() == Some(role));
    blocks.iter().position(|b| match b {
        Block::Section { blocks, .. } => blocks.iter().any(has),
        b => has(b),
    })
}

/// Decision and block pairs a page may weigh to find the block near a decision that became no
/// block; past it such a decision stands at the page's last block.
const NEAR_WORK: usize = 4_000_000;

/// The hosts of [`hosts`]: the block of each decision, the decisions that became rows of a
/// tagged table begun on an earlier page with the table, and the reasons of the table readers
/// by decision.
pub(super) type Hosts = (Vec<u32>, Vec<(usize, usize)>, BTreeMap<usize, Vec<String>>);

/// The block each of the page's `runs` runs (by its id) was written in, [`NO_BLOCK`] for a
/// run written in none: the block the placed part that holds it became (`placed`, what each
/// part became and its runs), else the first block that lists it among the runs laid out in
/// each block (`built`). A run a table reader cut stands in both: its id is the table's (the
/// reader that cut it), its piece left is text.
pub(super) fn run_blocks(built: &Built, placed: &[(Became, Vec<u32>)], runs: usize) -> Vec<u32> {
    let mut out = vec![NO_BLOCK; runs];
    let mut set = |id: u32, k: usize| {
        if let Some(slot) = out.get_mut(id as usize)
            && *slot == NO_BLOCK
        {
            *slot = u32::try_from(k).unwrap_or(NO_BLOCK);
        }
    };
    for (b, ids) in placed {
        if let Became::Block(k) = b {
            for &id in ids {
                set(id, *k);
            }
        }
    }
    for (k, ids) in built.block_ids.iter().enumerate() {
        for &id in ids {
            set(id, k);
        }
    }
    out
}

/// The block each decision of `decided` became, as its region's host:
/// - a running header or footer and a footnote legend: the first block of the role they were
///   built with (`page-header`, `page-footer`, `footnote`);
/// - any other: the block that holds most of its runs (`run_blocks`, see [`run_blocks`]).
///
/// A decision with nothing to write (no run with ink, no picture or code: a region the page
/// states over blank runs or none) became no block and stands at the page ([`NO_BLOCK`]),
/// unless it is a figure or a code. Any other
/// decision that became no block takes the block that follows it, as a picture is placed: the
/// decision with a block nearest to its top centre, the next block when that one stands above
/// it, else that block; when no decision has a block, the page's last block, and [`NO_BLOCK`]
/// on a page with no block. Also gives the decisions that became rows of a tagged table begun
/// on an earlier page, with the table, and the reasons the table readers gave for the runs of
/// each decision they did not read (`doubts`, by run id).
pub(super) fn hosts(
    decided: &Decided,
    page: &PageIn,
    built: &Built,
    placed: &[(Became, Vec<u32>)],
    run_blocks: &[u32],
    doubts: &HashMap<u32, &str>,
    tagged: bool,
) -> Hosts {
    let blocks = &built.blocks;
    let mut earlier_of: HashMap<u32, usize> = HashMap::new();
    for (b, ids) in placed {
        if let Became::Earlier(t) = b {
            for &id in ids {
                earlier_of.entry(id).or_insert(*t);
            }
        }
    }
    let block_of = |id: u32| -> Option<usize> {
        run_blocks
            .get(id as usize)
            .filter(|&&b| b != NO_BLOCK)
            .map(|&b| b as usize)
    };
    let headers = first_with_role(blocks, "page-header");
    let footers = first_with_role(blocks, "page-footer");
    let notes = first_with_role(blocks, "footnote");
    let mut earlier = Vec::new();
    let mut unsure: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    let mut hosts: Vec<Option<usize>> = Vec::with_capacity(decided.decisions.len());
    for k in 0..decided.decisions.len() {
        let runs = || {
            decided.prims(k).iter().filter_map(|p| match *p {
                PrimId::Text(i) => Some(i),
                _ => None,
            })
        };
        for id in runs() {
            if let Some(why) = doubts.get(&id) {
                let all = unsure.entry(k).or_default();
                if !all.iter().any(|w| w == why) {
                    all.push((*why).to_string());
                }
            }
        }
        if decided.gone[k] {
            hosts.push(None);
            continue;
        }
        hosts.push(match decided.class(k, tagged) {
            Class::Header => headers,
            Class::Footer => footers,
            Class::Legend => notes.or_else(|| most(runs().filter_map(block_of))),
            _ => match most(runs().filter_map(block_of)) {
                Some(b) => Some(b),
                None => {
                    if let Some(t) = runs().find_map(|i| earlier_of.get(&i)) {
                        earlier.push((k, *t));
                    }
                    None
                }
            },
        });
    }
    // The decisions with nothing to write: no run with ink, no picture or code. A figure or a
    // code (a drawing typed as a figure) is placed as a picture is.
    let blank: Vec<bool> = (0..decided.decisions.len())
        .map(|k| {
            !matches!(decided.decisions[k].kind, Kind::Figure | Kind::Code)
                && !decided.prims(k).iter().any(|p| match *p {
                    PrimId::Text(i) => page
                        .texts
                        .get(i as usize)
                        .is_some_and(|t| !t.text.trim().is_empty()),
                    PrimId::Picture(_) | PrimId::Code(_) => true,
                    _ => false,
                })
        })
        .collect();
    // The decisions with a block, to place the others near.
    let bounds = |k: usize| decided.bounds(k).filter(|_| !decided.gone[k]);
    let with_block: Vec<(Bounds, usize)> = hosts
        .iter()
        .enumerate()
        .filter_map(|(k, h)| Some((bounds(k)?, (*h)?)))
        .collect();
    let last = blocks.len().checked_sub(1);
    let mut work = NEAR_WORK;
    let hosts = hosts
        .into_iter()
        .enumerate()
        .map(|(k, h)| {
            let host = match (h, bounds(k)) {
                _ if blank[k] => None,
                (Some(b), _) => Some(b),
                (None, Some(at)) if work >= with_block.len() => {
                    work -= with_block.len();
                    near(at, &with_block, blocks.len()).or(last)
                }
                _ => last,
            };
            host.map_or(NO_BLOCK, |b| u32::try_from(b).unwrap_or(NO_BLOCK))
        })
        .collect();
    (hosts, earlier, unsure)
}

/// The block most of `blocks` name (the first of them on a tie).
fn most(blocks: impl Iterator<Item = usize>) -> Option<usize> {
    let mut count: Vec<(usize, usize)> = Vec::new();
    for b in blocks {
        match count.iter_mut().find(|c| c.0 == b) {
            Some(c) => c.1 += 1,
            None => count.push((b, 1)),
        }
    }
    let top = count.iter().map(|c| c.1).max()?;
    count.iter().find(|c| c.1 == top).map(|c| c.0)
}

/// The block that follows a region at `b` among `count` blocks, by the regions with a block
/// (`placed`, their bounds and block): the one nearest to the top centre of `b`, then the
/// block after it when it stands above that point (the last block when none is after it), else
/// its own block. Ties go to the earlier block.
fn near(b: Bounds, placed: &[(Bounds, usize)], count: usize) -> Option<usize> {
    let (x, y) = ((b.x0 + b.x1) / 2.0, b.y1);
    let distance = |q: &Bounds| {
        let dx = (q.x0 - x).max(x - q.x1).max(0.0);
        let dy = (q.y0 - y).max(y - q.y1).max(0.0);
        dx * dx + dy * dy
    };
    let (q, k) = placed
        .iter()
        .min_by(|(a, ka), (b, kb)| distance(a).total_cmp(&distance(b)).then(ka.cmp(kb)))?;
    let above = (q.y0 + q.y1) / 2.0 > y;
    Some(if above { (k + 1).min(count - 1) } else { *k })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rezzer_model::Inline;

    fn at(top: f32) -> Bounds {
        Bounds {
            x0: 40.0,
            y0: top - 12.0,
            x1: 300.0,
            y1: top,
        }
    }

    /// A region with no block of its own stands at the block that follows it: the block of
    /// the region nearest above it is passed for the next one; with no region near, none.
    #[test]
    fn a_region_with_no_block_stands_at_the_block_that_follows() {
        let placed = [(at(800.0), 0), (at(600.0), 1)];
        assert_eq!(near(at(700.0), &placed, 2), Some(1));
        assert_eq!(near(at(590.0), &placed, 2), Some(1));
        assert_eq!(near(at(700.0), &[], 2), None);
    }

    /// What a placed part became: its table, else its first block that is no found-by note;
    /// rows of a table an earlier page began build none.
    #[test]
    fn a_placed_part_becomes_its_first_block() {
        let note = Block::Paragraph {
            inlines: vec![Inline::plain("Found by: ...")],
            role: Some("found-by".into()),
            anchor: None,
        };
        let blocks = vec![
            Block::paragraph(vec![Inline::plain("Muster GmbH")]),
            note,
            Block::paragraph(vec![Inline::plain("Erika Mustermann")]),
        ];
        assert_eq!(became(&blocks, 1, None), Became::Block(2));
        assert_eq!(became(&blocks, 3, Some(4)), Became::Earlier(4));
        assert_eq!(became(&blocks, 3, None), Became::Nothing);
    }
}
