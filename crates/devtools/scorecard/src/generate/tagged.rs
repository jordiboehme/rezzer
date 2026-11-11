// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Facts from the tags of a tagged PDF. The file is read with its tags; the blocks the tag
//! tree states (regions the file states, certainty `exact`) give the truth: the rows of its
//! tables, its headings, the order of its paragraphs. The same file read with its tags taken
//! out (see [`crate::strip`]) is then scored against them: the layout engine on real pages
//! with known answers. Blocks the reader found by layout give no facts.

use std::collections::{HashMap, HashSet};

use rezzer_model::{Certainty, Document, Evidence, RegionKind};

use super::{CAP, distinct, lead, sample};
use crate::check::check;
use crate::doc::{Unit, View};
use crate::facts::{Fact, Term};
use crate::text::{count_words, has_words};

/// The evidence id of a region the file states.
const STATED: &str = "given.file_states";

/// The longest cell text written whole; a longer one is named by its first words.
const WHOLE: usize = 80;

/// The text of a cell as a fact names it: whole when it is short, else its first words.
fn cell_term(text: &str) -> Term {
    if text.chars().count() <= WHOLE {
        Term::exact(text)
    } else {
        Term::contains(&lead(text, 6))
    }
}

/// The facts the tags of `doc` give; none when the file is not tagged. `doc` is the read of
/// the file with its tags.
pub fn tagged(doc: &Document) -> Vec<Fact> {
    if doc.properties.facts.tagged != Some(true) {
        return Vec::new();
    }
    let view = View::of(doc);
    // The kind of each region the file states, by anchor.
    let mut stated: HashMap<&str, RegionKind> = HashMap::new();
    for region in &doc.regions {
        let by_file = region
            .evidence
            .iter()
            .any(|e| matches!(e, Evidence::Rule { id, .. } if id == STATED));
        if region.certainty == Certainty::Exact && by_file {
            stated
                .entry(region.anchor.0.as_str())
                .or_insert(region.kind);
        }
    }
    let kind_of = |unit: &Unit| unit.anchors.iter().find_map(|a| stated.get(a.as_str()));
    let holders = |text: &str| {
        view.units
            .iter()
            .filter(|u| has_words(&u.text, text))
            .count()
    };

    let (mut rows, mut under, mut apart) = (Vec::new(), Vec::new(), Vec::new());
    for table in &view.tables {
        let is_stated = table
            .anchor
            .as_deref()
            .is_some_and(|a| stated.get(a) == Some(&RegionKind::Table));
        if !is_stated {
            continue;
        }
        let filled = |r: usize| -> Vec<&Unit> {
            table.rows[r]
                .iter()
                .map(|p| &view.units[p.unit])
                .filter(|u| !u.text.is_empty())
                .collect()
        };
        for r in 0..table.rows.len() {
            let cells = filled(r);
            if cells.len() < 2 {
                continue;
            }
            rows.push(Fact::row(
                cells.iter().take(8).map(|u| cell_term(&u.text)).collect(),
            ));
            // The first cell of this row and the last of the next never share a row, when
            // each text stands in one place of the file only.
            if r + 1 < table.rows.len() {
                let next = filled(r + 1);
                // A cell that spans both rows stands in both.
                let in_both = |u: &Unit| {
                    cells.iter().any(|c| std::ptr::eq(*c, u))
                        && next.iter().any(|c| std::ptr::eq(*c, u))
                };
                if let (Some(a), Some(b)) = (cells.first(), next.last())
                    && next.len() >= 2
                    && !in_both(a)
                    && !in_both(b)
                    && a.text != b.text
                    && a.text.chars().count() <= WHOLE
                    && b.text.chars().count() <= WHOLE
                    && holders(&a.text) == 1
                    && holders(&b.text) == 1
                {
                    apart.push(Fact::not_row(Term::exact(&a.text), Term::exact(&b.text)));
                }
            }
        }
        if table.header_rows == 0 || table.header_rows > table.rows.len() {
            continue;
        }
        let heads = &table.rows[table.header_rows - 1];
        for row in &table.rows[table.header_rows..] {
            for cell in row {
                let value = &view.units[cell.unit].text;
                let head = heads
                    .iter()
                    .find(|h| h.unit != cell.unit && h.first <= cell.first && cell.end <= h.end)
                    .map(|h| &view.units[h.unit].text);
                // A value that stands in more than one cell of the table gives no fact: it
                // would hold under a wrong header as well.
                let once = table
                    .rows
                    .iter()
                    .flatten()
                    .map(|p| p.unit)
                    .collect::<HashSet<usize>>()
                    .into_iter()
                    .filter(|u| view.units[*u].text == *value)
                    .count()
                    == 1;
                if let Some(head) = head
                    && once
                    && !head.is_empty()
                    && !value.is_empty()
                    && head.chars().count() <= WHOLE
                    && value.chars().count() <= WHOLE
                {
                    under.push(Fact::cell_under(Term::exact(head), Term::exact(value)));
                }
            }
        }
    }

    let mut headings = Vec::new();
    let mut seen = HashSet::new();
    // Blocks of running text the file states, in reading order: each with the first words
    // that stand in one place of the file only, to find it again by.
    let mut flow: Vec<String> = Vec::new();
    let mut texts = Vec::new();
    for unit in &view.units {
        if unit.table.is_some() || unit.text.is_empty() {
            continue;
        }
        let Some(kind) = kind_of(unit) else { continue };
        match kind {
            RegionKind::Heading if unit.heading.is_some() => {
                if unit.text.chars().count() <= 200 && seen.insert(unit.text.as_str()) {
                    headings.push(Fact::heading(&unit.text, None));
                }
            }
            RegionKind::Paragraph | RegionKind::List => {
                let words = unit.text.split(' ').count();
                if words >= 3 {
                    texts.push(Fact::text(Term::contains(&lead(&unit.text, 10))));
                }
            }
            _ => continue,
        }
        let start = lead(&unit.text, 6);
        if start.contains(' ') && start.len() >= 8 && count_words(&view.stream, &start) == 1 {
            flow.push(start);
        }
    }
    let order: Vec<Fact> = flow
        .windows(2)
        .map(|w| Fact::order(Term::contains(&w[0]), Term::contains(&w[1])))
        .collect();

    let mut links = Vec::new();
    let mut targets = HashSet::new();
    for (target, _) in &view.links {
        let outside = ["http://", "https://", "mailto:"]
            .iter()
            .any(|p| target.starts_with(p));
        if outside && targets.insert(target.as_str()) {
            links.push(Fact::link(target));
        }
    }

    // The read the facts come from holds each of them. A fact it does not hold (two cells of
    // one text, a text that the tags state twice) says nothing about the file and is left out.
    let mut out = Vec::new();
    for list in [rows, apart, under, headings, order, texts, links] {
        let mut list = distinct(list);
        list.retain(|fact| check(&view, fact).pass);
        out.extend(sample(list, CAP));
    }
    distinct(out)
}
