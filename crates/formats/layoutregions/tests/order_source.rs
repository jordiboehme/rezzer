// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Fix round 1 of Task 37, M4: the source order against the order it gave before Task 37,
//! when each loose region looked at every sourced region one by one (`Cx::by_source` at
//! 6601ca13). 4,000 seeded pages of 2 to 31 regions in 1 to 4 columns, with loose pictures
//! and codes, text regions of pieces side by side and rules, and a source order as made or
//! with two places swapped. The orders are folded into one number; the number below was
//! made by this file at 6601ca13, with the default work bound and with none. A change of
//! the search that moves any region in any of the 8,000 orders changes it.
use layoutregions::*;

struct Dice(u64);
impl Dice {
    fn roll(&mut self, n: u32) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) % u64::from(n)) as u32
    }
}

fn run(text: &str, x: f32, y: f32, w: f32, size: f32, line: u32) -> TextRun {
    TextRun {
        text: text.to_string(),
        x,
        y,
        w,
        h: size,
        size,
        font: 1,
        font_kind: FontKind::Proportional,
        bold: false,
        italic: false,
        color: None,
        shift: Shift::Base,
        line,
        off_axis: false,
        gaps: Vec::new(),
    }
}

fn region(id: u32, kind: Kind, prims: Vec<PrimId>, b: Bounds) -> Region {
    Region {
        id: RegionId(id),
        page: 1,
        kind,
        bounds: b,
        prims,
        certainty: Certainty::Rule,
        score: Score(0),
        signals: Vec::new(),
        alternatives: Vec::new(),
        parent: None,
        unsure: Vec::new(),
        host: 0,
    }
}

/// The orders of the pages below folded into one number, as made at 6601ca13.
const ORDERS_ONE_BY_ONE: u64 = 51_162_702_753_020_041;

#[test]
fn the_source_order_is_the_order_of_the_walk_of_every_sourced_region() {
    let mut totals = [0u64; 3];
    let mut cases = 0;
    for seed in 0..4000u64 {
        let mut d = Dice(seed * 7919 + 13);
        let cols = 1 + d.roll(4);
        let colw = 500.0 / cols as f32;
        let mut page = PageIn {
            page: 1,
            size: [612.0, 792.0],
            turn: 0,
            texts: Vec::new(),
            rules: Vec::new(),
            boxes: Vec::new(),
            pictures: Vec::new(),
            codes: Vec::new(),
            given: Vec::new(),
        };
        let mut regions: Vec<Region> = Vec::new();
        let mut source: Vec<RegionId> = Vec::new();
        let n = 2 + d.roll(30);
        let mut line = 0u32;
        for _ in 0..n {
            let c = d.roll(cols) as f32;
            let wide = d.roll(6) == 0;
            let x0 = 50.0 + c * colw + d.roll(3) as f32 * 4.0;
            let w = if wide {
                colw * (1 + d.roll(cols)) as f32 - 10.0
            } else {
                colw * (0.3 + 0.2 * d.roll(4) as f32)
            };
            let y1 = 100.0 + 10.0 * d.roll(60) as f32;
            let h = 10.0 * (1 + d.roll(6)) as f32;
            let b = Bounds {
                x0,
                y0: y1 - h,
                x1: x0 + w,
                y1,
            };
            let id = regions.len() as u32;
            match d.roll(4) {
                0 => {
                    // A loose picture or code.
                    let kind = if d.roll(2) == 0 {
                        Kind::Figure
                    } else {
                        Kind::Code
                    };
                    let prim = if kind == Kind::Figure {
                        page.pictures.push(PictureIn { bounds: b });
                        PrimId::Picture(page.pictures.len() as u32 - 1)
                    } else {
                        page.codes.push(CodeIn { bounds: b });
                        PrimId::Code(page.codes.len() as u32 - 1)
                    };
                    regions.push(region(id, kind, vec![prim], b));
                }
                _ => {
                    // A text region of a few lines, some of pieces side by side, some with a rule.
                    let mut prims = Vec::new();
                    let lines = 1 + d.roll(5);
                    for k in 0..lines {
                        let y = y1 - 9.0 - (h / lines as f32) * k as f32;
                        let pieces = 1 + d.roll(3);
                        for q in 0..pieces {
                            let px = x0 + (w / pieces as f32) * q as f32;
                            page.texts.push(run(
                                "Muster",
                                px,
                                y,
                                (w / pieces as f32) * 0.8,
                                8.0,
                                line,
                            ));
                            prims.push(PrimId::Text(page.texts.len() as u32 - 1));
                        }
                        line += 1;
                    }
                    if d.roll(5) == 0 {
                        page.rules.push(RuleIn {
                            x0,
                            y0: y1 - h,
                            x1: x0 + w,
                            y1: y1 - h,
                            thick: 0.5,
                            dashed: false,
                            rect: false,
                        });
                        prims.push(PrimId::Rule(page.rules.len() as u32 - 1));
                    }
                    let kind = [
                        Kind::Paragraph,
                        Kind::Table,
                        Kind::Spatial,
                        Kind::LabelValue,
                    ][d.roll(4) as usize];
                    regions.push(region(id, kind, prims, b));
                    if d.roll(8) != 0 {
                        source.push(RegionId(id));
                    }
                }
            }
        }
        // The source order: as made, or shuffled a little.
        if d.roll(3) == 0 && source.len() > 2 {
            let a = d.roll(source.len() as u32) as usize;
            let b = d.roll(source.len() as u32) as usize;
            source.swap(a, b);
        }
        for (slot, per) in [None, Some(u64::MAX), Some(4u64)].into_iter().enumerate() {
            let got = match per {
                Some(per) => with_place_work(per, || order_by_source(&page, &regions, &source)),
                None => order_by_source(&page, &regions, &source),
            };
            for (k, r) in got.iter().enumerate() {
                totals[slot] = totals[slot]
                    .wrapping_mul(1_000_003)
                    .wrapping_add(u64::from(r.0) + 31 * k as u64 + seed);
            }
            cases += 1;
        }
    }
    assert_eq!(cases, 12_000);
    assert_eq!(totals[0], ORDERS_ONE_BY_ONE, "the default work bound");
    assert_eq!(totals[1], ORDERS_ONE_BY_ONE, "no work bound");
    // With 4 steps an item the walk runs out of work on many pages and places the rest by
    // their height: the order then depends on what the walk charges, so it is not pinned.
    assert_ne!(totals[2], 0);
}
