// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Facts from the tags of a tagged PDF. The file is read with its tags; the blocks the tag
//! tree states (regions the file states, certainty `exact`) give the truth: the rows of its
//! tables, its headings, the order of its paragraphs within one flow. The same file read with its tags taken
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

/// Where a block stands and what the order facts need of it: its page (from 1), where it
/// reaches from and to across the page and from its bottom to its top, in the frame the page
/// is stored in (`y` upward), and what its text says of the blocks around it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Place {
    page: u32,
    x: [f32; 2],
    y: [f32; 2],
    /// It is a heading.
    heading: bool,
    /// Its text ends in a colon: a label.
    label: bool,
    /// Its text ends in the middle of a sentence (on a letter, a digit, a comma or a hyphen).
    runs_on: bool,
    /// Its text starts with a small letter: it goes on a sentence.
    goes_on: bool,
}

impl Place {
    /// What the text of a block says: see the fields.
    fn with_text(mut self, heading: bool, text: &str) -> Place {
        let text = text.trim();
        self.heading = heading;
        self.label = text.ends_with(':');
        self.runs_on = text
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || matches!(c, ',' | '-'));
        self.goes_on = text.chars().next().is_some_and(char::is_lowercase);
        self
    }

    fn width(&self) -> f32 {
        self.x[1] - self.x[0]
    }

    /// Whether this share ([`ROW_SHARE`]) of its height, or more, lies within the height of
    /// `other`: it stands on the rows of `other`.
    fn on_rows_of(&self, other: &Place) -> bool {
        let shared = self.y[1].min(other.y[1]) - self.y[0].max(other.y[0]);
        let h = self.y[1] - self.y[0];
        h > 0.0 && shared >= ROW_SHARE * h
    }
}

/// Two blocks of a page are one flow when this share of the width of the narrower one, or
/// more, stands over or under the other. The same test, with the same share, as the order
/// check of the metamorphic tests (`one_flow` in `layoutregions/tests/meta`).
const FLOW_SHARE: f32 = 0.5;

/// Two columns are of one set when the narrower one has this share of the width of the
/// other, or more. The same share as `COLUMN_SHARE` of the metamorphic tests: the columns of
/// a set are set to one width, a note in the margin of a column is not.
const COLUMN_SHARE: f32 = 0.8;

/// A block stands on the rows of another when this share of its height, or more, lies within
/// the height of the other. The same share as `ROW_SHARE` of the metamorphic tests.
const ROW_SHARE: f32 = 0.5;

/// Whether the blocks at `a` and `b` are one flow: one stands over the other (a heading over
/// its section, the paragraphs of one column, a block across the page over the columns under
/// it).
fn one_flow(a: Place, b: Place) -> bool {
    let shared = a.x[1].min(b.x[1]) - a.x[0].max(b.x[0]);
    let narrow = a.width().min(b.width());
    a.page == b.page && narrow > 0.0 && shared >= FLOW_SHARE * narrow
}

/// Whether `b`, the block the tags name right after `a` on its page, is held to `a` though it
/// stands beside it, so that their order carries meaning:
///
/// - a sentence runs from `a` into `b`: `a` ends in the middle of a sentence, `b` starts
///   with a small letter and stands right of `a`, starting above its end (the head of the
///   next column);
/// - `a` is the foot of a column and `b` the head of the next column of its set: both are
///   running text, `b` stands right of `a` at a like width ([`COLUMN_SHARE`]) and starts
///   above the end of `a`. When `b` is a heading it opens the next part of the same flow,
///   and the like width is that of `under`: the next block the tags name in the flow of
///   `b`, running text that starts under the top of `b`, right of `a`;
/// - `a` is a heading left of `b`, on its rows ([`ROW_SHARE`]), and `b` is as wide as `a` or
///   wider: a side heading in the margin of its block. A title beside a narrower block (a
///   number box, a date) heads nothing there;
/// - `a` is a label (its text ends in a colon) left of its value `b`, each on the rows of
///   the other.
///
/// Two other blocks side by side (a head block and a date box, a margin note and a line of a
/// form, the columns of a footer) do not depend on each other: a reader may read either
/// first, and no fact states their order. The tags' own sign of a flow, the structure parent
/// of a block, is not in the read; these tests stand in for it. A caption and its picture
/// give no fact here: an order fact names two texts, and a picture has none.
fn held(a: Place, b: Place, under: Option<Place>) -> bool {
    if a.page != b.page || one_flow(a, b) || b.x[0] <= a.x[0] {
        return false;
    }
    let above_end = b.y[1] > a.y[0];
    let sentence = !a.heading && !b.heading && a.runs_on && b.goes_on && above_end;
    // The text of the column that `b` heads: `b` itself, or what stands under the heading.
    let body = if b.heading {
        under.filter(|u| {
            !u.heading && one_flow(b, *u) && u.y[1] <= b.y[1] && !one_flow(a, *u) && u.x[0] > a.x[0]
        })
    } else {
        Some(b)
    };
    let alike = |u: Place| a.width().min(u.width()) >= COLUMN_SHARE * a.width().max(u.width());
    let column = !a.heading && above_end && body.is_some_and(alike);
    let heading = a.heading && !b.heading && a.on_rows_of(&b) && b.width() >= a.width();
    let field = a.label && !b.heading && a.on_rows_of(&b) && b.on_rows_of(&a);
    sentence || column || heading || field
}

/// The next block after the one at `i` that is of one flow with it ([`one_flow`], so on its
/// page): blocks beside that flow that the tags name between the two are passed over.
fn next_of_flow(places: &[Option<Place>], i: usize) -> Option<Place> {
    let at = places.get(i).copied().flatten()?;
    places[i + 1..]
        .iter()
        .flatten()
        .take_while(|b| b.page == at.page)
        .find(|b| one_flow(at, **b))
        .copied()
}

/// The pairs of blocks whose order a fact states, as indices into `places` (the blocks in
/// reading order, each with its place when the read gives one):
///
/// - each block with the next block of its page that is of one flow with it ([`one_flow`]);
///   blocks beside that flow that the tags name between the two are passed over;
/// - each block with the block the tags name right after it, when that one stands beside it
///   and is held to it ([`held`]): the head of the next column (also a heading over its
///   text), the block of a side heading, the value of a label;
/// - the last block of a page with the first of the next page that has one: pages are read
///   in order.
///
/// A block without a place takes part in no pair. The bounds are those of the page as it is
/// stored: on a page whose text is read in a turned frame the tests look along the other
/// axis, so such a page gives the facts of blocks side by side and none of a flow. The
/// model does not say how a page is turned.
fn follows(places: &[Option<Place>]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (i, a) in places.iter().enumerate() {
        let Some(a) = a else { continue };
        if let Some(Some(b)) = places.get(i + 1)
            && held(*a, *b, next_of_flow(places, i + 1))
        {
            out.push((i, i + 1));
        }
        let later = places[i + 1..]
            .iter()
            .enumerate()
            .filter_map(|(k, b)| b.map(|b| (i + 1 + k, b)));
        let mut next_page = None;
        let mut found = None;
        for (j, b) in later {
            if b.page != a.page {
                next_page = Some(j);
                break;
            }
            if one_flow(*a, b) {
                found = Some(j);
                break;
            }
        }
        // Only the last block of a page is paired with the next page.
        let last = !places[i + 1..].iter().flatten().any(|b| b.page == a.page);
        match (found, next_page) {
            (Some(j), _) => out.push((i, j)),
            (None, Some(j)) if last => out.push((i, j)),
            _ => {}
        }
    }
    out
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
    // Where each of them stands: its page and its bounds.
    let mut places: HashMap<&str, Place> = HashMap::new();
    for region in &doc.regions {
        let by_file = region
            .evidence
            .iter()
            .any(|e| matches!(e, Evidence::Rule { id, .. } if id == STATED));
        if region.certainty == Certainty::Exact && by_file {
            stated
                .entry(region.anchor.0.as_str())
                .or_insert(region.kind);
            places.entry(region.anchor.0.as_str()).or_insert(Place {
                page: region.page,
                x: [region.bounds[0], region.bounds[2]],
                y: [region.bounds[1], region.bounds[3]],
                ..Place::default()
            });
        }
    }
    let kind_of = |unit: &Unit| unit.anchors.iter().find_map(|a| stated.get(a.as_str()));
    let place_of = |unit: &Unit| {
        unit.anchors
            .iter()
            .find(|a| stated.contains_key(a.as_str()))
            .and_then(|a| places.get(a.as_str()))
            .copied()
    };
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
    let mut flow: Vec<(String, Option<Place>)> = Vec::new();
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
            let heading = *kind == RegionKind::Heading;
            flow.push((
                start,
                place_of(unit).map(|p| p.with_text(heading, &unit.text)),
            ));
        }
    }
    let order: Vec<Fact> = follows(&flow.iter().map(|(_, p)| *p).collect::<Vec<_>>())
        .into_iter()
        .map(|(a, b)| Fact::order(Term::contains(&flow[a].0), Term::contains(&flow[b].0)))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A block of running text from `x0` to `x1`, twenty points high, that is a whole
    /// sentence.
    fn at(page: u32, x0: f32, x1: f32) -> Option<Place> {
        Some(block(
            page,
            [x0, x1],
            [600.0, 620.0],
            false,
            "A whole line.",
        ))
    }

    /// [`held`] of two blocks with nothing named after them.
    fn held2(a: Place, b: Place) -> bool {
        held(a, b, None)
    }

    fn block(page: u32, x: [f32; 2], y: [f32; 2], heading: bool, text: &str) -> Place {
        Place {
            page,
            x,
            y,
            ..Place::default()
        }
        .with_text(heading, text)
    }

    /// Order facts stand within one flow only: a note beside a column is passed over, the
    /// lines of the column follow each other, and the pages follow each other.
    #[test]
    fn order_is_stated_within_one_flow_only() {
        let places = [
            // A title across the page, two paragraphs of the left column with a margin
            // note named between them, and a head block beside the column, under its end.
            at(1, 50.0, 550.0),
            at(1, 50.0, 280.0),
            at(1, 20.0, 45.0),
            at(1, 50.0, 280.0),
            Some(block(
                1,
                [320.0, 420.0],
                [500.0, 520.0],
                false,
                "A side box.",
            )),
            // The next page.
            at(2, 50.0, 550.0),
        ];
        assert_eq!(follows(&places), [(0, 1), (1, 3), (4, 5)]);
        // A block without a place is in no pair, and nothing pairs with it.
        let places = [at(1, 50.0, 280.0), None, at(1, 50.0, 280.0)];
        assert_eq!(follows(&places), [(0, 2)]);
    }

    /// Relation (a) across a column break: the foot of a column and the head of the next
    /// column of its set, which starts above that foot. The share of a like width is moved
    /// both ways; a column under the end of the left one, and one to the left, are free.
    #[test]
    fn the_foot_of_a_column_is_followed_by_the_head_of_the_next() {
        let text = "A whole line.";
        let foot = block(1, [50.0, 250.0], [100.0, 300.0], false, text);
        let head = |x1: f32, y: [f32; 2]| block(1, [300.0, x1], y, false, text);
        assert!(held2(foot, head(500.0, [500.0, 700.0])));
        assert_eq!(
            follows(&[Some(foot), Some(head(500.0, [500.0, 700.0]))]),
            [(0, 1)]
        );
        // 161 of 200 points is a like width, 159 is not.
        assert!(held2(foot, head(461.0, [500.0, 700.0])));
        assert!(!held2(foot, head(459.0, [500.0, 700.0])));
        assert_eq!(COLUMN_SHARE, 0.8);
        // Under the end of the left column, to the left, on another page, a heading.
        assert!(!held2(foot, head(500.0, [20.0, 90.0])));
        assert!(!held2(head(500.0, [500.0, 700.0]), foot));
        assert!(!held2(
            foot,
            Place {
                page: 2,
                ..head(500.0, [500.0, 700.0])
            }
        ));
        assert!(!held2(
            foot,
            block(1, [300.0, 500.0], [500.0, 700.0], true, text)
        ));
    }

    /// Relation (a) when the next column starts with a heading: the heading opens the next
    /// part of the same flow, and the like width is that of the text under the heading. A
    /// heading with nothing under it, with another heading or a narrow block under it, or
    /// under the end of the left column, is free.
    #[test]
    fn the_foot_of_a_column_is_followed_by_the_heading_of_the_next() {
        let text = "A whole line.";
        let foot = block(1, [50.0, 250.0], [100.0, 300.0], false, text);
        let head = block(1, [300.0, 350.0], [688.0, 700.0], true, "Results");
        let under = |x1: f32, heading: bool| block(1, [300.0, x1], [500.0, 680.0], heading, text);
        assert!(held(foot, head, Some(under(500.0, false))));
        assert_eq!(
            follows(&[Some(foot), Some(head), Some(under(500.0, false))]),
            [(0, 1), (1, 2)]
        );
        // A block beside the columns that the tags name between the heading and its text is
        // passed over: the text is the next block of the flow of the heading.
        let note = block(1, [520.0, 560.0], [600.0, 620.0], false, text);
        assert_eq!(
            follows(&[
                Some(foot),
                Some(head),
                Some(note),
                Some(under(500.0, false))
            ]),
            [(0, 1), (1, 3)]
        );
        // 161 of 200 points is a like width, 159 is not.
        assert!(held(foot, head, Some(under(461.0, false))));
        assert!(!held(foot, head, Some(under(459.0, false))));
        // Nothing under the heading, a heading under it, text over it.
        assert!(!held(foot, head, None));
        assert_eq!(follows(&[Some(foot), Some(head)]), []);
        assert!(!held(foot, head, Some(under(500.0, true))));
        let over = block(1, [300.0, 500.0], [710.0, 760.0], false, text);
        assert!(!held(foot, head, Some(over)));
        // The heading under the end of the left column, and the text on the next page.
        let low = block(1, [300.0, 350.0], [80.0, 92.0], true, "Results");
        let low_text = block(1, [300.0, 500.0], [20.0, 70.0], false, text);
        assert!(!held(foot, low, Some(low_text)));
        assert_eq!(
            follows(&[
                Some(foot),
                Some(head),
                Some(Place {
                    page: 2,
                    ..under(500.0, false)
                })
            ]),
            [(1, 2)]
        );
    }

    /// A side heading stands in the margin of a block that is as wide as it or wider. A
    /// title beside a narrower block (a number box) heads nothing there: a free pair. The
    /// width is moved both ways.
    #[test]
    fn a_title_beside_a_number_box_is_free() {
        let title = block(
            1,
            [100.0, 280.0],
            [748.0, 760.0],
            true,
            "Sample Works Parts Return",
        );
        let number = block(
            1,
            [356.0, 421.0],
            [748.0, 755.0],
            false,
            "Sheet No. 4411-0074",
        );
        assert!(!held2(title, number));
        assert_eq!(follows(&[Some(title), Some(number)]), []);
        // A side heading of two lines beside a block of two lines that ends where it ends.
        let side = block(1, [36.0, 71.0], [231.0, 251.0], true, "Sum Due");
        let lines = |x1: f32| block(1, [115.0, x1], [231.0, 249.0], false, "Take the sum.");
        assert!(held2(side, lines(470.0)));
        // 35 points of heading: a block of 35 points is headed, one of 34 is not.
        assert!(held2(side, lines(150.0)));
        assert!(!held2(side, lines(149.0)));
        // A known limit (owner: after 0.1.0; a limit of the truth, it changes no read): a short title
        // beside a wider box of one line on its row. The tags give no kinds here, and geometry
        // cannot tell a number line from a block of one line, so the pair is held though it is
        // free. When a sign tells the two apart, this assertion turns. Part 2b-1 of Task 31c found
        // none in the tags.
        let short = block(1, [100.0, 135.0], [748.0, 760.0], true, "Invoice");
        let wide = block(
            1,
            [356.0, 451.0],
            [748.0, 755.0],
            false,
            "Sheet No. 4411-0074",
        );
        assert!(held2(short, wide));
    }

    /// Relations (b), (c) and (e) between blocks side by side: a side heading and its block,
    /// a label and its value on one row, a sentence that runs into a narrower block at the
    /// head of the next column. The same blocks without the sign are free.
    #[test]
    fn a_side_heading_a_label_and_a_sentence_hold_the_next_block() {
        let body = |text: &str| block(1, [200.0, 500.0], [600.0, 660.0], false, text);
        let left = |y0: f32, heading: bool, text: &str| {
            block(1, [50.0, 120.0], [y0, y0 + 10.0], heading, text)
        };
        // (b): half the height of the heading on the rows of the block, moved both ways.
        assert!(held2(left(640.0, true, "Sum due"), body("Take the sum.")));
        assert!(held2(left(655.0, true, "Sum due"), body("Take the sum.")));
        assert!(!held2(left(656.0, true, "Sum due"), body("Take the sum.")));
        assert_eq!(ROW_SHARE, 0.5);
        // The free case: the same two blocks, neither a heading nor a label.
        assert!(!held2(
            left(640.0, false, "Sample Works"),
            body("Take the sum.")
        ));
        // (c): a label and its value, each on the rows of the other.
        let value = |y0: f32| block(1, [200.0, 300.0], [y0, y0 + 10.0], false, "Sample part");
        assert!(held2(left(640.0, false, "Name:"), value(640.0)));
        assert!(held2(left(640.0, false, "Name:"), value(635.0)));
        assert!(!held2(left(640.0, false, "Name:"), value(634.0)));
        assert!(!held2(left(640.0, false, "Name"), value(640.0)));
        // A label beside a tall block is on its rows, but the block is not on the label's.
        assert!(!held2(left(640.0, false, "Name:"), body("Take the sum.")));
        // (e): the text runs on into a block that starts with a small letter.
        let part = left(640.0, false, "held by four screws, and each of");
        assert!(held2(part, body("them takes a washer.")));
        assert!(!held2(part, body("Each takes a washer.")));
        assert!(!held2(
            left(640.0, false, "held by four screws."),
            body("them takes a washer.")
        ));
    }

    /// The share of one flow, moved both ways, and at twice the size.
    #[test]
    fn one_flow_is_half_the_narrower_width() {
        let p = |x0: f32, x1: f32| Place {
            page: 1,
            x: [x0, x1],
            ..Place::default()
        };
        for k in [1.0f32, 2.0] {
            assert!(one_flow(p(0.0, 100.0 * k), p(79.5 * k, 119.5 * k)));
            assert!(!one_flow(p(0.0, 100.0 * k), p(80.5 * k, 120.5 * k)));
        }
        assert!(!one_flow(p(0.0, 100.0), p(100.0, 200.0)));
        // Another page is another flow.
        assert!(!one_flow(
            p(0.0, 100.0),
            Place {
                page: 2,
                x: [0.0, 100.0],
                ..Place::default()
            }
        ));
    }
}
