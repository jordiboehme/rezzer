// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The one order of the runs of a page the engine reads them in.
//!
//! The segmentation, the typers and the arbitration read the runs of a [`PageIn`] in the
//! order they stand in: the runs of one line next to each other, a line from its first run
//! to its last, and where two primitives are equal by every other measure the lower id
//! first. A host whose runs stand in another order (the order of a file's content stream,
//! runs put in by a later step) would get a read that depends on that order.
//! [`RunOrder::of`] gives the page with its runs in the order of their places, which is the
//! same for every order they are handed in: a host segments, types and arbitrates that page
//! and names the runs by its own ids again with [`RunOrder::back`].
//!
//! The first key of that order is the line number of a run ([`TextRun::line`]), which the
//! host gives: it must be the number of the line in reading order. The sort only orders the
//! runs inside one line, by their left edges. A host that leaves the line number at one
//! value gets the runs of the whole page by their left edges, column by column. A line the
//! host hands in several runs that do not read left to right (text turned up or upside
//! down, each run marked off its axis) stands in the order of its places too, which is not
//! the order it reads in; the typers skip such runs.
//!
//! This order is not the reading order of the output. It decides which runs make a segment
//! and what kind the segment is. The host places the regions and reads the text of a region
//! kept as laid out by its own flow.

use crate::file::{Legend, RefPlace};
use crate::input::{PageIn, PrimId};
use crate::line::{in_place_order, place_order};
use crate::segment::Segments;
use crate::work;

/// A page with its runs in the order of their places, and the id each of them had.
#[derive(Debug, Clone, PartialEq)]
pub struct RunOrder {
    /// The page: its runs by line and left edge, the run ids of its given regions renamed.
    pub page: PageIn,
    /// For each run of `page`, its id on the page handed in.
    pub back: Vec<u32>,
    /// For each run of the page handed in, its id on `page`.
    pub to: Vec<u32>,
}

impl RunOrder {
    /// `page` with its runs in the order of their places (see the module doc and
    /// [`place_order`]), or `None` when they stand in that order already. The ids are a tie break
    /// only. Sorts the runs once,
    /// and its work is counted.
    pub fn of(page: &PageIn) -> Option<RunOrder> {
        let n = page.texts.len();
        work::add(n as u64);
        if in_place_order(&page.texts) {
            return None;
        }
        let mut back: Vec<u32> = (0..n as u32).collect();
        // A stable sort: the id is the last key.
        back.sort_by(|&a, &b| place_order(&page.texts[a as usize], &page.texts[b as usize]));
        work::add(n as u64 * u64::from(usize::BITS - n.leading_zeros()));
        let mut to = vec![0; n];
        for (new, &old) in back.iter().enumerate() {
            to[old as usize] = new as u32;
        }
        let mut out = page.clone();
        out.texts = back
            .iter()
            .map(|&i| page.texts[i as usize].clone())
            .collect();
        for g in &mut out.given {
            for p in &mut g.prims {
                if let PrimId::Text(i) = p
                    && let Some(&t) = to.get(*i as usize)
                {
                    *i = t;
                }
            }
        }
        Some(RunOrder {
            page: out,
            back,
            to,
        })
    }

    /// The primitives `prims` of [`RunOrder::page`] by the ids of the page handed in, sorted.
    pub fn back(&self, prims: &mut [PrimId]) {
        for p in prims.iter_mut() {
            if let PrimId::Text(i) = p
                && let Some(&b) = self.back.get(*i as usize)
            {
                *i = b;
            }
        }
        prims.sort_unstable();
    }

    /// `legend`, which names its runs by the ids of the page handed in, by the ids of
    /// [`RunOrder::page`]: the runs of its footnotes (sorted), its raised numbers and the
    /// words they go to. The typers look the legend up on the page in place order (see
    /// [`crate::typers::PageFacts::legend`]). Copies this one legend and nothing else of the
    /// file, and its work is counted: a step for each run it names.
    pub fn legend(&self, legend: &Legend) -> Legend {
        let to = |run: &mut u32| *run = self.to.get(*run as usize).copied().unwrap_or(*run);
        let mut out = legend.clone();
        let mut named = out.lines.len() + 2 * out.raised.len();
        for e in &mut out.entries {
            named += e.runs.len();
            e.runs.iter_mut().for_each(to);
            e.runs.sort_unstable();
        }
        for r in &mut out.raised {
            to(&mut r.run);
            match &mut r.place {
                RefPlace::End => {}
                RefPlace::Inside { word, .. } | RefPlace::Above { word } => to(word),
            }
        }
        work::add(1 + named as u64);
        out
    }

    /// The segments of [`RunOrder::page`] with their primitives by the ids of the page handed
    /// in (see [`RunOrder::back`]).
    pub fn segments_back(&self, segs: &mut Segments) {
        for s in &mut segs.all {
            self.back(&mut s.prims);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{FontKind, GivenRegion, Shift, TextRun};
    use crate::region::Kind;

    fn run(text: &str, x: f32, y: f32, line: u32) -> TextRun {
        TextRun {
            text: text.to_string(),
            x,
            y,
            w: 20.0,
            h: 10.0,
            size: 10.0,
            font: 0,
            font_kind: FontKind::Unknown,
            bold: false,
            italic: false,
            color: None,
            shift: Shift::Base,
            line,
            off_axis: false,
            gaps: Vec::new(),
        }
    }

    fn page(texts: Vec<TextRun>, given: Vec<GivenRegion>) -> PageIn {
        PageIn {
            page: 0,
            size: [600.0, 800.0],
            turn: 0,
            texts,
            rules: Vec::new(),
            boxes: Vec::new(),
            pictures: Vec::new(),
            codes: Vec::new(),
            given,
        }
    }

    #[test]
    fn runs_in_the_order_of_their_places_are_left_as_they_are() {
        let p = page(
            vec![
                run("north", 50.0, 700.0, 0),
                run("gate", 90.0, 700.0, 0),
                // A raised mark after its word: same line, farther right.
                run("2", 111.0, 704.0, 0),
                run("south", 50.0, 688.0, 1),
            ],
            Vec::new(),
        );
        assert_eq!(RunOrder::of(&p), None);
        assert_eq!(RunOrder::of(&page(Vec::new(), Vec::new())), None);
    }

    #[test]
    fn every_order_of_the_runs_gives_one_page() {
        let texts = vec![
            run("north", 50.0, 700.0, 0),
            run("gate", 90.0, 700.0, 0),
            run("south", 50.0, 688.0, 1),
            run("wall", 90.0, 688.0, 1),
            run("east", 50.0, 676.0, 2),
        ];
        let given = |ids: [u32; 2]| GivenRegion {
            kind: Kind::Paragraph,
            prims: ids.iter().map(|&i| PrimId::Text(i)).collect(),
            order: 0,
            level: None,
            role: None,
            group: None,
        };
        let base = page(texts.clone(), vec![given([2, 3])]);
        assert_eq!(RunOrder::of(&base), None);
        // Every one of the 120 orders: the same page, and each id leads back to its run.
        let mut order = [0usize, 1, 2, 3, 4];
        let mut seen = 0;
        permute(&mut order, 0, &mut |o| {
            let at = |old: u32| o.iter().position(|&i| i as u32 == old).unwrap() as u32;
            let p = page(
                o.iter().map(|&i| texts[i].clone()).collect(),
                vec![given([at(2), at(3)])],
            );
            seen += 1;
            let Some(c) = RunOrder::of(&p) else {
                assert_eq!(
                    o,
                    [0, 1, 2, 3, 4],
                    "only the order of the places is left alone"
                );
                return;
            };
            assert_eq!(c.page, base, "order {o:?}");
            for (new, &old) in c.back.iter().enumerate() {
                assert_eq!(p.texts[old as usize], c.page.texts[new]);
                assert_eq!(c.to[old as usize], new as u32);
            }
            let mut prims = vec![PrimId::Text(4), PrimId::Rule(0), PrimId::Text(0)];
            c.back(&mut prims);
            let mut want = vec![
                PrimId::Text(c.back[4]),
                PrimId::Rule(0),
                PrimId::Text(c.back[0]),
            ];
            want.sort_unstable();
            assert_eq!(prims, want);
        });
        assert_eq!(seen, 120);
    }

    fn permute(o: &mut [usize; 5], k: usize, f: &mut impl FnMut([usize; 5])) {
        if k == o.len() {
            f(*o);
            return;
        }
        for i in k..o.len() {
            o.swap(k, i);
            permute(o, k + 1, f);
            o.swap(k, i);
        }
    }

    #[test]
    fn the_id_decides_only_between_runs_in_one_place() {
        // Two runs in one place with one text: they keep the order they came in. A third
        // that differs in its baseline alone does not: the higher one is first.
        let p = page(
            vec![
                run("mark", 50.0, 700.0, 1),
                run("mark", 50.0, 700.0, 1),
                run("mark", 50.0, 704.0, 1),
                run("head", 50.0, 720.0, 0),
            ],
            Vec::new(),
        );
        let c = RunOrder::of(&p).expect("the head line is last");
        assert_eq!(c.back, [3, 2, 0, 1]);
        // The line counts before the place on the page: a line the reader numbers first is
        // first, wherever it stands.
        let p = page(
            vec![run("low", 50.0, 100.0, 0), run("high", 50.0, 700.0, 1)],
            Vec::new(),
        );
        assert_eq!(RunOrder::of(&p), None);
    }

    /// A legend that names its runs by the ids of the page handed in is renamed to the ids
    /// of the page in place order, and the work counted is that of the one legend.
    #[test]
    fn a_legend_is_renamed_to_the_ids_of_the_placed_page() {
        use crate::file::{LegendEntry, NoteMark, RaisedRef};
        // Handed in: the legend line first, then the raised number before its word.
        let handed = page(
            vec![
                run("2 per month", 40.0, 70.0, 2),
                run("2", 62.0, 702.0, 0),
                run("charge", 40.0, 700.0, 0),
            ],
            Vec::new(),
        );
        let order = RunOrder::of(&handed).expect("out of place order");
        assert_eq!(order.to, [2, 1, 0]);
        let entry = |runs: Vec<u32>| LegendEntry {
            mark: NoteMark::Raised(2),
            cut: 1,
            x: 40.0,
            top: 70.0,
            last: 70.0,
            h: 10.0,
            runs,
        };
        let raised = |run, word| RaisedRef {
            run,
            mark: NoteMark::Raised(2),
            place: RefPlace::Above { word },
        };
        let legend = Legend {
            lines: vec![2],
            entries: vec![entry(vec![0])],
            raised: vec![raised(1, 2)],
        };
        let before = work::work_steps();
        let placed = order.legend(&legend);
        let steps = work::work_steps().zip(before).map(|(a, b)| a - b);
        assert_eq!(
            placed,
            Legend {
                lines: vec![2],
                entries: vec![entry(vec![2])],
                raised: vec![raised(1, 0)],
            }
        );
        // One step, and one for each line, run and reference with its word it names.
        if let Some(steps) = steps {
            assert_eq!(steps, 1 + 1 + 1 + 2);
        }
    }
}
