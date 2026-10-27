// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Splitter: packing specs into units and the cost of what they hold.

use super::*;

impl<'a> Splitter<'a> {
    /// The size of `item` with `blocks` as its content.
    pub(super) fn blocks_cost(&self, item: &Item<'a>, blocks: Vec<Block>) -> usize {
        let mut probe = item.clone();
        probe.content = Content::Owned(blocks);
        self.cost(&probe)
    }

    /// Whether every block of `item` went to another unit (a column profile).
    pub(super) fn all_moved(&self, item: &Item<'a>) -> bool {
        let list = item.content.blocks();
        !self.moved.is_empty()
            && !list.is_empty()
            && list
                .iter()
                .all(|b| self.moved.contains(&std::ptr::from_ref(b)))
    }

    /// The final packing (plan Task 5: units packed greedily while they fit): neighbouring
    /// units of one top-level unit, inside the same sections, are joined in document order
    /// while the part stays within the limit, whatever level made them. So a heading's piece
    /// goes with the rows after it, a table's last rows with the notes after them, and a
    /// small rest of a section with the section after it. A unit cut as text stays alone.
    pub(super) fn pack(&self, specs: Vec<Spec<'a>>) -> Vec<Spec<'a>> {
        let mut out: Vec<Spec<'a>> = Vec::with_capacity(specs.len());
        for mut s in specs {
            if s.pieces == 1 && s.cost.is_none() {
                s.cost = Some(self.cost(&s.item));
            }
            if let Some(prev) = out.last_mut()
                && let (Some(a), Some(b)) = (prev.cost, s.cost)
                && prev.pieces == 1
                && s.pieces == 1
                && a + 2 + b <= self.budget
                && let Some(joined) = self.join(prev, &s)
            {
                let cost = self.cost(&joined.item);
                if cost <= self.budget {
                    *prev = Spec {
                        cost: Some(cost),
                        ..joined
                    };
                    continue;
                }
            }
            out.push(s);
        }
        out
    }

    /// `a` and `b` as one unit, `None` when they lie in different top-level units. Inside
    /// the sections both lie in, each keeps the sections only it lies in, written around its
    /// blocks. A table's rows keep their name when the other one is a piece of the unit that
    /// holds the table (its heading, its notes); else the part names every unit it holds
    /// (ruling P5-AH).
    fn join(&self, a: &Spec<'a>, b: &Spec<'a>) -> Option<Spec<'a>> {
        let (x, y) = (&a.item, &b.item);
        if x.top != y.top {
            return None;
        }
        let common = x
            .wraps
            .iter()
            .zip(&y.wraps)
            .take_while(|(p, q)| p == q)
            .count();
        let content = match (&x.content, &y.content) {
            (
                Content::Slice { list, from, to },
                Content::Slice {
                    list: other,
                    from: next,
                    to: end,
                },
            ) if std::ptr::eq(*list, *other)
                && to == next
                && x.wraps.len() == common
                && y.wraps.len() == common =>
            {
                Content::Slice {
                    list,
                    from: *from,
                    to: *end,
                }
            }
            _ => {
                let mut blocks = self.nested(x, common);
                blocks.extend(self.nested(y, common));
                Content::Owned(blocks)
            }
        };
        let mut anchors = anchor_runs(a);
        anchors.extend(anchor_runs(b));
        let mut wrap_anchors = a.wrap_anchors.clone();
        wrap_anchors.extend(x.wraps[common..].iter().filter_map(|w| w.anchor));
        wrap_anchors.extend(b.wrap_anchors.iter().copied());
        wrap_anchors.extend(y.wraps[common..].iter().filter_map(|w| w.anchor));
        let mut item = x.clone();
        item.wraps.truncate(common);
        item.content = content;
        item.last = y.last;
        item.pages = match (x.pages, y.pages) {
            (Some((p, _)), Some((_, q))) => Some((p, q)),
            _ => None,
        };
        item.rezzer = x.rezzer && y.rezzer;
        item.page_names = false;
        item.heading_only = false;
        item.lead = 0;
        let (held_x, held_y) = (x.held(), y.held());
        let holds_only = |rows: &Item<'_>, other: &[String]| {
            rows.table
                .as_ref()
                .is_some_and(|t| other.iter().all(|h| h == t))
        };
        // A piece on pages past the rows' pages says so in the location.
        let within = |piece: &Item<'_>, rows: &Item<'_>| match (piece.pages, rows.on_pages()) {
            (None, _) => true,
            (Some((a, b)), Some((c, d))) => a >= c && b <= d,
            (Some(_), None) => false,
        };
        let both = joined_location(&x.shown_location(), &y.shown_location());
        // A piece on pages past the rows' pages is named too, in title and location:
        // `GLOSSARY, rows 266 to 278 and pages 422 to 450`.
        let rows_and_pages = capped(
            &format!("{} and {}", x.shown_location(), y.shown_location()),
            TITLE_CAP,
        );
        let suffix = if y.table.is_none() && holds_only(x, &held_y) {
            if !within(y, x) {
                item.title = rows_and_pages.clone();
                item.location = rows_and_pages;
            }
            a.suffix.clone()
        } else if x.table.is_none() && holds_only(y, &held_x) {
            item.base = y.base.clone();
            item.title = y.title.clone();
            if within(x, y) {
                item.location = y.location.clone();
            } else {
                item.title = rows_and_pages.clone();
                item.location = rows_and_pages;
            }
            item.paragraphs = None;
            item.family = y.family;
            item.table = y.table.clone();
            item.held = y.held.clone();
            b.suffix.clone()
        } else {
            let mut held = held_x;
            for h in held_y {
                if held.last() != Some(&h) {
                    held.push(h);
                }
            }
            item.table = None;
            item.paragraphs = match (x.paragraphs, y.paragraphs) {
                (Some((p, _)), Some((_, q))) if x.location == y.location => Some((p, q)),
                _ => None,
            };
            if item.paragraphs.is_none() {
                item.location = both;
            }
            if x.family.is_none() || x.family != y.family {
                item.family = None;
                item.title = capped(&listed(&held), TITLE_CAP);
            }
            item.held = if held.len() > 1 { held } else { Vec::new() };
            a.suffix.clone()
        };
        Some(Spec {
            item,
            suffix,
            pieces: 1,
            anchors,
            wrap_anchors,
            cost: None,
        })
    }

    /// The blocks of `item` without the moved ones, inside the sections of its wraps from
    /// `common` on, as [`wrapped`] writes them: a section's anchor only where the item
    /// reaches its end.
    fn nested(&self, item: &Item<'a>, common: usize) -> Vec<Block> {
        let mut blocks: Vec<Block> = item
            .content
            .blocks()
            .iter()
            .filter(|b| !self.moved.contains(&std::ptr::from_ref(*b)))
            .cloned()
            .collect();
        for w in item.wraps[common..].iter().rev() {
            blocks = vec![Block::Section {
                kind: w.kind,
                title: w.title.map(str::to_string),
                anchor: w
                    .anchor
                    .filter(|_| item.last)
                    .map(|a| rezzer_model::Anchor(a.to_string())),
                blocks,
            }];
        }
        blocks
    }

    // -----------------------------------------------------------------------------------
    // Writing
}
