// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The reading order of a page's regions: furniture first and last, the given order kept, and
//! the XY order over the region boxes for the rest.

use layoutregions::segment::bounds_of;
use layoutregions::*;

/// The body text size of the pages here.
const BODY: f32 = 10.0;

/// A page and its regions, built up region by region. Regions are numbered in the order they
/// are made; [`Page::ask`] hands them to [`order`] in a scrambled order.
struct Page {
    page: PageIn,
    regions: Vec<Region>,
}

impl Page {
    fn new() -> Page {
        Page {
            page: PageIn {
                page: 0,
                size: [600.0, 800.0],
                turn: 0,
                texts: Vec::new(),
                rules: Vec::new(),
                boxes: Vec::new(),
                pictures: Vec::new(),
                codes: Vec::new(),
                given: Vec::new(),
            },
            regions: Vec::new(),
        }
    }

    /// `n` lines of `chars` characters at `x`, the first baseline at `top`, of `size` points,
    /// one line pitch of 1.2 sizes.
    fn lines(&mut self, x: f32, top: f32, n: usize, chars: usize, size: f32) -> Vec<PrimId> {
        (0..n)
            .map(|k| {
                let i = self.page.texts.len() as u32;
                let text: String = "x".repeat(chars);
                self.page.texts.push(TextRun {
                    text,
                    x,
                    y: top - 1.2 * size * k as f32,
                    w: chars as f32 * size * 0.5,
                    h: size,
                    size,
                    font: 1,
                    font_kind: FontKind::Proportional,
                    bold: false,
                    italic: false,
                    color: None,
                    shift: Shift::Base,
                    line: i,
                    off_axis: false,
                    gaps: Vec::new(),
                });
                PrimId::Text(i)
            })
            .collect()
    }

    fn picture(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) -> PrimId {
        let i = self.page.pictures.len() as u32;
        self.page.pictures.push(PictureIn {
            bounds: Bounds { x0, y0, x1, y1 },
        });
        PrimId::Picture(i)
    }

    /// A horizontal rule from `x0` to `x1` at `y`.
    fn rule(&mut self, x0: f32, x1: f32, y: f32) -> PrimId {
        let i = self.page.rules.len() as u32;
        self.page.rules.push(RuleIn {
            x0,
            y0: y,
            x1,
            y1: y,
            thick: 0.5,
            dashed: false,
            rect: false,
        });
        PrimId::Rule(i)
    }

    /// A region of `kind` over `prims`, its bounds theirs.
    fn region(&mut self, kind: Kind, prims: Vec<PrimId>) -> RegionId {
        let bounds = bounds_of(&self.page, &prims).expect("a region holds something");
        self.region_in(kind, prims, bounds)
    }

    /// A region of `kind` over `prims` with the given bounds.
    fn region_in(&mut self, kind: Kind, prims: Vec<PrimId>, bounds: Bounds) -> RegionId {
        let id = RegionId(self.regions.len() as u32);
        self.regions.push(Region {
            id,
            page: 0,
            kind,
            bounds,
            prims,
            certainty: Certainty::Rule,
            score: Score(800),
            signals: Vec::new(),
            alternatives: Vec::new(),
            parent: None,
            unsure: Vec::new(),
            host: id.0,
        });
        id
    }

    /// A paragraph of `n` lines.
    fn para(&mut self, x: f32, top: f32, n: usize, chars: usize) -> RegionId {
        let prims = self.lines(x, top, n, chars, BODY);
        self.region(Kind::Paragraph, prims)
    }

    /// The regions in a scrambled order (every third first, then the others reversed), so
    /// that the order made is never the order handed over.
    fn scrambled(&self) -> Vec<Region> {
        let mut out: Vec<Region> = self.regions.iter().step_by(3).cloned().collect();
        out.extend(
            self.regions
                .iter()
                .enumerate()
                .filter(|(k, _)| k % 3 != 0)
                .rev()
                .map(|(_, r)| r.clone()),
        );
        out
    }

    fn ask(&self) -> Vec<RegionId> {
        let regions = self.scrambled();
        let got = order(&self.page, &regions);
        assert_once(&regions, &got);
        got
    }

    /// The page with every length times `f`.
    fn scaled(&self, f: f32) -> Page {
        let b = |b: Bounds| Bounds {
            x0: b.x0 * f,
            y0: b.y0 * f,
            x1: b.x1 * f,
            y1: b.y1 * f,
        };
        let mut page = self.page.clone();
        page.size = [page.size[0] * f, page.size[1] * f];
        for t in &mut page.texts {
            t.x *= f;
            t.y *= f;
            t.w *= f;
            t.h *= f;
            t.size *= f;
        }
        for r in &mut page.rules {
            r.x0 *= f;
            r.y0 *= f;
            r.x1 *= f;
            r.y1 *= f;
            r.thick *= f;
        }
        for x in &mut page.boxes {
            x.bounds = b(x.bounds);
        }
        for x in &mut page.pictures {
            x.bounds = b(x.bounds);
        }
        for x in &mut page.codes {
            x.bounds = b(x.bounds);
        }
        let regions = self
            .regions
            .iter()
            .map(|r| Region {
                bounds: b(r.bounds),
                ..r.clone()
            })
            .collect();
        Page { page, regions }
    }
}

/// Every region of `regions` stands once in `got`, and nothing else does.
fn assert_once(regions: &[Region], got: &[RegionId]) {
    let mut want: Vec<RegionId> = regions.iter().map(|r| r.id).collect();
    let mut have = got.to_vec();
    want.sort_unstable();
    have.sort_unstable();
    assert_eq!(have, want, "every region once");
}

/// A title over two columns of two paragraphs each. `gap` is the white space between the
/// paragraphs of a column; those of the two columns stand at the same heights.
fn two_columns(gap: f32) -> (Page, [RegionId; 5]) {
    let mut p = Page::new();
    let t = p.lines(60.0, 760.0, 1, 40, 16.0);
    let title = p.region(Kind::Heading, t);
    let pitch = 1.2 * BODY;
    let second = 730.0 - 5.0 * pitch - gap;
    let l1 = p.para(50.0, 730.0, 5, 44);
    let l2 = p.para(50.0, second, 5, 44);
    let r1 = p.para(320.0, 730.0, 5, 44);
    let r2 = p.para(320.0, second, 5, 44);
    (p, [title, l1, l2, r1, r2])
}

#[test]
fn two_columns_read_down_each() {
    let (p, want) = two_columns(6.0);
    assert_eq!(p.ask(), want);
    // The white space between the paragraphs is wider than the one under the title, and lines
    // up across the columns: the columns still read down each.
    let (p, want) = two_columns(30.0);
    assert_eq!(p.ask(), want);
}

#[test]
fn comparison_is_one_region() {
    let mut p = Page::new();
    let l = p.para(50.0, 760.0, 6, 44);
    let r = p.para(320.0, 760.0, 6, 44);
    // A comparison across both columns, its rows of short cells on shared baselines.
    let mut cells = Vec::new();
    for x in [50.0, 200.0, 320.0, 470.0] {
        cells.extend(p.lines(x, 660.0, 5, 8, BODY));
    }
    let table = p.region(Kind::Table, cells);
    let after = p.para(50.0, 560.0, 3, 100);
    assert_eq!(p.ask(), [l, r, table, after]);
}

#[test]
fn head_left_then_right() {
    let mut p = Page::new();
    let address = p.para(50.0, 720.0, 4, 24);
    // The date stands beside the second line of the address.
    let d = p.lines(420.0, 720.0 - 1.2 * BODY, 1, 10, BODY);
    let date = p.region(Kind::Paragraph, d);
    let letter = p.para(50.0, 640.0, 8, 100);
    assert_eq!(p.ask(), [address, date, letter]);
}

/// Task 22: labels in a column beside their values in a column, and under them, further left,
/// a pair with nothing beside it. The pair is no part of the labels' column (it stands under
/// none of them): it reads after both columns, not between the labels and their values.
#[test]
fn a_lone_region_off_its_side_ends_the_columns() {
    let mut p = Page::new();
    let l = p.lines(300.0, 470.0, 2, 8, BODY);
    let labels = p.region(Kind::Paragraph, l);
    let v = p.lines(380.0, 466.0, 2, 10, BODY);
    let values = p.region(Kind::Paragraph, v);
    let below = p.lines(60.0, 420.0, 1, 30, BODY);
    let pair = p.region(Kind::LabelValue, below);
    assert_eq!(p.ask(), [labels, values, pair]);
    // The last paragraph of a left column under the end of the right column stays in its
    // column: it stands under the left column's paragraphs.
    let mut p = Page::new();
    let l1 = p.para(50.0, 760.0, 6, 44);
    let r1 = p.para(320.0, 760.0, 4, 44);
    let l2 = p.para(50.0, 670.0, 3, 44);
    assert_eq!(p.ask(), [l1, l2, r1]);
}

#[test]
fn figure_takes_its_place() {
    let mut p = Page::new();
    let first = p.para(50.0, 760.0, 4, 44);
    let pic = p.picture(60.0, 600.0, 250.0, 700.0);
    let figure = p.region(Kind::Figure, vec![pic]);
    let second = p.para(50.0, 580.0, 4, 44);
    // A column on the right beside both.
    let right = p.para(320.0, 760.0, 16, 44);
    assert_eq!(p.ask(), [first, figure, second, right]);
}

#[test]
fn furniture_first_and_last() {
    let mut p = Page::new();
    let h = p.lines(50.0, 780.0, 1, 30, 8.0);
    let header = p.region(Kind::HeaderBand, h);
    let h = p.lines(400.0, 780.0, 1, 10, 8.0);
    let header_right = p.region(Kind::HeaderBand, h);
    let body = p.para(50.0, 740.0, 10, 100);
    let n = p.lines(50.0, 90.0, 2, 80, 7.0);
    let legend = p.region(Kind::FootnoteLegend, n);
    let f = p.lines(280.0, 30.0, 1, 6, 8.0);
    let footer = p.region(Kind::FooterBand, f);
    // A body paragraph under the legend still stands before it.
    let low = p.para(50.0, 60.0, 1, 40);
    assert_eq!(p.ask(), [header, header_right, body, low, legend, footer]);
}

#[test]
fn footer_columns_read_down_each() {
    // A footer of three columns of short lines on shared baselines: two lines on the left,
    // four in the middle, three on the right. One footer band per column: each column whole.
    let mut p = Page::new();
    let body = p.para(50.0, 760.0, 10, 100);
    let mut cols = Vec::new();
    for (x, n) in [(50.0, 2), (230.0, 4), (410.0, 3)] {
        let l = p.lines(x, 80.0, n, 20, 7.0);
        cols.push(p.region(Kind::FooterBand, l));
    }
    assert_eq!(p.ask(), [body, cols[0], cols[1], cols[2]]);
    // The same footer with one region per line: still column by column, never alternating.
    let mut p = Page::new();
    let body = p.para(50.0, 760.0, 10, 100);
    let mut want = vec![body];
    for (x, n) in [(50.0, 2), (230.0, 4), (410.0, 3)] {
        for k in 0..n {
            let l = p.lines(x, 80.0 - 8.4 * k as f32, 1, 20, 7.0);
            want.push(p.region(Kind::FooterBand, l));
        }
    }
    assert_eq!(p.ask(), want);
    // As body regions (a footer the engine did not take as furniture), the same.
    let mut p = Page::new();
    let body = p.para(50.0, 760.0, 10, 100);
    let mut want = vec![body];
    for (x, n) in [(50.0, 2), (230.0, 4), (410.0, 3)] {
        for k in 0..n {
            let l = p.lines(x, 80.0 - 8.4 * k as f32, 1, 20, 7.0);
            want.push(p.region(Kind::Paragraph, l));
        }
    }
    assert_eq!(p.ask(), want);
}

#[test]
fn given_order_holds() {
    let mut p = Page::new();
    // A tagged page: the file reads the right column before the left one.
    let left = p.para(50.0, 760.0, 6, 44);
    let right = p.para(320.0, 760.0, 6, 44);
    let pic = p.picture(60.0, 500.0, 540.0, 600.0);
    // An untagged figure under both columns, and a tagged paragraph under it.
    let figure = p.region(Kind::Figure, vec![pic]);
    let last = p.para(50.0, 480.0, 3, 100);
    for (order, id) in [(0, right), (1, left), (2, last)] {
        let prims = p.regions[id.0 as usize].prims.clone();
        p.page.given.push(GivenRegion {
            kind: Kind::Paragraph,
            prims,
            order,
            level: None,
            role: None,
            group: None,
        });
        let r = &mut p.regions[id.0 as usize];
        r.certainty = Certainty::Exact;
        r.signals.push(Signal {
            id: "given.file_states",
            detail: "paragraph".to_string(),
            weight: 1000,
        });
    }
    assert_eq!(p.ask(), [right, left, figure, last]);
}

/// Marks the regions `ids` as stated by the file, in that order.
fn give(p: &mut Page, ids: &[RegionId]) {
    for (order, id) in ids.iter().enumerate() {
        let prims = p.regions[id.0 as usize].prims.clone();
        p.page.given.push(GivenRegion {
            kind: Kind::Paragraph,
            prims,
            order: order as u32,
            level: None,
            role: None,
            group: None,
        });
        let r = &mut p.regions[id.0 as usize];
        r.certainty = Certainty::Exact;
        r.signals.push(Signal {
            id: "given.file_states",
            detail: "paragraph".to_string(),
            weight: 1000,
        });
    }
}

#[test]
fn untagged_regions_go_with_their_neighbours() {
    // The file reads the right column first. An untagged figure between the two paragraphs of
    // the left column stays between them, and an untagged title above all stays first.
    let mut p = Page::new();
    let t = p.lines(60.0, 780.0, 1, 40, 16.0);
    let title = p.region(Kind::Heading, t);
    let l1 = p.para(50.0, 740.0, 4, 44);
    let pic = p.picture(60.0, 600.0, 250.0, 690.0);
    let figure = p.region(Kind::Figure, vec![pic]);
    let l2 = p.para(50.0, 580.0, 4, 44);
    let right = p.para(320.0, 740.0, 16, 44);
    give(&mut p, &[right, l1, l2]);
    assert_eq!(p.ask(), [title, right, l1, figure, l2]);
}

#[test]
fn an_untagged_figure_under_the_columns_stays_under_them() {
    // The file reads the right column first; an untagged figure across the page under both
    // columns, no tagged region after it.
    let mut p = Page::new();
    let left = p.para(50.0, 760.0, 6, 44);
    let right = p.para(320.0, 760.0, 6, 44);
    let pic = p.picture(60.0, 500.0, 540.0, 600.0);
    let figure = p.region(Kind::Figure, vec![pic]);
    give(&mut p, &[right, left]);
    assert_eq!(p.ask(), [right, left, figure]);
}

#[test]
fn an_untagged_figure_ends_its_column() {
    // The file reads the right column first; an untagged figure at the foot of the left
    // column stays after the left column's text, never at the top of the page.
    let mut p = Page::new();
    let l1 = p.para(50.0, 760.0, 6, 44);
    let pic = p.picture(60.0, 600.0, 250.0, 680.0);
    let figure = p.region(Kind::Figure, vec![pic]);
    let right = p.para(320.0, 760.0, 16, 44);
    give(&mut p, &[right, l1]);
    assert_eq!(p.ask(), [right, l1, figure]);
}

#[test]
fn an_untagged_figure_keeps_to_its_column_text() {
    // The file reads the right column, then the left one, then a tagged block above both. An
    // untagged figure at the foot of the left column goes with the left column's text, not
    // after the block the file reads last.
    let mut p = Page::new();
    let top = p.para(50.0, 780.0, 2, 100);
    let l1 = p.para(50.0, 740.0, 6, 44);
    let pic = p.picture(60.0, 580.0, 250.0, 660.0);
    let figure = p.region(Kind::Figure, vec![pic]);
    let right = p.para(320.0, 740.0, 16, 44);
    give(&mut p, &[right, l1, top]);
    assert_eq!(p.ask(), [right, l1, figure, top]);
}

/// A tagged title across the page, two tagged columns and an untagged figure across the page
/// under both; with `note`, a tagged note across the page under the figure. The file reads the
/// title, the right column, the left column, then the note.
fn title_columns_figure(note: bool) -> (Page, Vec<RegionId>) {
    let mut p = Page::new();
    let t = p.lines(60.0, 780.0, 1, 60, 16.0);
    let title = p.region(Kind::Heading, t);
    let left = p.para(50.0, 740.0, 6, 44);
    let right = p.para(320.0, 740.0, 6, 44);
    let pic = p.picture(60.0, 500.0, 540.0, 640.0);
    let figure = p.region(Kind::Figure, vec![pic]);
    let mut file = vec![title, right, left];
    if note {
        let n = p.para(50.0, 480.0, 2, 100);
        file.push(n);
    }
    give(&mut p, &file);
    let mut want = vec![title, right, left, figure];
    want.extend(file.get(3));
    (p, want)
}

#[test]
fn an_untagged_figure_under_tagged_columns_stays_under_them() {
    let (p, want) = title_columns_figure(false);
    assert_eq!(p.ask(), want);
    // A tagged note under the figure: the figure stays between the columns and the note.
    let (p, want) = title_columns_figure(true);
    assert_eq!(p.ask(), want);
}

#[test]
fn an_untagged_figure_keeps_to_its_sub_column() {
    // The left column is tagged; the right column is an untagged figure beside tagged text.
    // The file reads the right column first: its text, then the figure with it, then the left.
    let mut p = Page::new();
    let left = p.para(50.0, 760.0, 12, 44);
    let pic = p.picture(320.0, 650.0, 400.0, 770.0);
    let figure = p.region(Kind::Figure, vec![pic]);
    let r = p.lines(420.0, 760.0, 10, 24, BODY);
    let text = p.region(Kind::Paragraph, r);
    give(&mut p, &[text, left]);
    assert_eq!(p.ask(), [text, figure, left]);
}

#[test]
fn an_untagged_heading_opens_its_column() {
    // The file reads the right column first; an untagged heading at the top of the left
    // column stays with the left column, not before the right one.
    let mut p = Page::new();
    let h = p.lines(50.0, 760.0, 1, 30, 14.0);
    let heading = p.region(Kind::Heading, h);
    let l1 = p.para(50.0, 730.0, 6, 44);
    let right = p.para(320.0, 760.0, 8, 44);
    give(&mut p, &[right, l1]);
    assert_eq!(p.ask(), [right, heading, l1]);
}

/// A page of every rule above: header, a title over two columns with a figure, a table, a
/// rule, a letter head and a footer of columns.
fn busy() -> Page {
    let mut p = Page::new();
    let h = p.lines(50.0, 785.0, 1, 30, 8.0);
    p.region(Kind::HeaderBand, h);
    let t = p.lines(60.0, 760.0, 1, 30, 16.0);
    p.region(Kind::Heading, t);
    p.para(50.0, 730.0, 5, 44);
    let pic = p.picture(60.0, 580.0, 250.0, 660.0);
    p.region(Kind::Figure, vec![pic]);
    p.para(50.0, 560.0, 3, 44);
    p.para(320.0, 730.0, 14, 44);
    let mut cells = Vec::new();
    for x in [50.0, 200.0, 320.0, 470.0] {
        cells.extend(p.lines(x, 480.0, 3, 8, BODY));
    }
    p.region(Kind::Table, cells);
    p.rule(40.0, 560.0, 430.0);
    p.para(50.0, 410.0, 4, 24);
    let d = p.lines(420.0, 398.0, 1, 10, BODY);
    p.region(Kind::Paragraph, d);
    p.para(50.0, 330.0, 8, 100);
    for (x, n) in [(50.0, 2), (230.0, 4), (410.0, 3)] {
        let l = p.lines(x, 80.0, n, 20, 7.0);
        p.region(Kind::FooterBand, l);
    }
    p
}

#[test]
fn order_is_scale_free() {
    let p = busy();
    let want = p.ask();
    let ids = |v: &[u32]| v.iter().map(|&k| RegionId(k)).collect::<Vec<_>>();
    assert_eq!(want, ids(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]));
    for f in [0.25, 0.5, 2.0, 3.7, 10.0] {
        assert_eq!(p.scaled(f).ask(), want, "at {f} times the size");
    }
}

#[test]
fn a_wide_rule_parts_the_columns() {
    // Two columns whose paragraphs stand at the same heights.
    let (mut p, [title, l1, l2, r1, r2]) = two_columns(20.0);
    let plain = p.ask();
    assert_eq!(plain, [title, l1, l2, r1, r2]);
    // A rule across both columns between the paragraphs: above it, then below it.
    let y = p.regions[l1.0 as usize].bounds.y0 - 10.0;
    p.rule(40.0, 560.0, y);
    assert_eq!(p.ask(), [title, l1, r1, l2, r2]);
    // A short rule parts nothing.
    let (mut p, _) = two_columns(20.0);
    p.rule(40.0, 200.0, y);
    assert_eq!(p.ask(), plain);
}

#[test]
fn a_box_gutter_waits_for_the_bands() {
    // Text at the top right, and under it a box on the left beside text on the right. The box
    // is drawn in one of four ways.
    #[derive(Clone, Copy)]
    enum Drawn {
        /// Not drawn: a plain paragraph.
        Not,
        /// Its rules belong to the region.
        Own,
        /// A drawn box of the page frames it, the region holds only its text.
        PageBox,
        /// Four rules of the page frame it, the region holds only its text.
        PageRules,
    }
    let build = |drawn: Drawn| {
        let mut p = Page::new();
        let above = p.para(320.0, 760.0, 3, 40);
        // The text spans 60 to 210 across and 644 to 690 up.
        let mut inner = p.lines(60.0, 680.0, 4, 30, BODY);
        let boxed = match drawn {
            Drawn::Not => p.region(Kind::Paragraph, inner),
            Drawn::Own => {
                inner.extend([p.rule(50.0, 250.0, 700.0), p.rule(50.0, 250.0, 620.0)]);
                p.region(Kind::LabelValue, inner)
            }
            Drawn::PageBox => {
                p.page.boxes.push(BoxIn {
                    bounds: Bounds {
                        x0: 57.0,
                        y0: 641.0,
                        x1: 213.0,
                        y1: 693.0,
                    },
                    filled: false,
                    stroked: true,
                    widget: false,
                });
                p.region(Kind::Paragraph, inner)
            }
            Drawn::PageRules => {
                p.rule(57.0, 213.0, 693.0);
                p.rule(57.0, 213.0, 641.0);
                for x in [57.0, 213.0] {
                    p.page.rules.push(RuleIn {
                        x0: x,
                        y0: 641.0,
                        x1: x,
                        y1: 693.0,
                        thick: 0.5,
                        dashed: false,
                        rect: false,
                    });
                }
                p.region(Kind::Paragraph, inner)
            }
        };
        let beside = p.para(320.0, 680.0, 4, 40);
        (p, [above, boxed, beside])
    };
    // A column of text on each side: the columns read down each.
    let (p, [above, boxed, beside]) = build(Drawn::Not);
    assert_eq!(p.ask(), [boxed, above, beside]);
    // Only a drawn box makes the gutter: the text above the box is read before it, whoever
    // holds the lines of the box.
    for drawn in [Drawn::Own, Drawn::PageBox, Drawn::PageRules] {
        let (p, [above, boxed, beside]) = build(drawn);
        assert_eq!(p.ask(), [above, boxed, beside]);
    }
}

#[test]
fn a_row_reads_left_to_right() {
    // Two regions closer than a gutter, the right one a little higher: one row, left first.
    let mut p = Page::new();
    let a = p.lines(50.0, 700.0, 1, 10, BODY);
    let label = p.region(Kind::Paragraph, a);
    let b = p.lines(105.0, 702.0, 2, 20, BODY);
    let value = p.region(Kind::Paragraph, b);
    let c = p.lines(50.0, 660.0, 1, 10, BODY);
    let next = p.region(Kind::Paragraph, c);
    assert_eq!(p.ask(), [label, value, next]);
}

#[test]
fn staggered_blocks_read_by_their_tops() {
    // A block on the right that stands mostly between two blocks on the left, beside the
    // lower one by less than a third of its height: no columns, each block by its top.
    let mut p = Page::new();
    let a = p.lines(50.0, 730.0, 3, 40, BODY);
    let a = p.region(Kind::Paragraph, a);
    let c = p.lines(320.0, 690.0, 7, 40, BODY);
    let c = p.region(Kind::Paragraph, c);
    let b = p.lines(50.0, 620.0, 4, 40, BODY);
    let b = p.region(Kind::Paragraph, b);
    assert_eq!(p.ask(), [a, c, b]);
}

#[test]
fn children_follow_their_parent() {
    let mut p = Page::new();
    let top = p.para(50.0, 760.0, 3, 80);
    let a = p.lines(60.0, 700.0, 2, 30, BODY);
    let b = p.lines(320.0, 700.0, 2, 30, BODY);
    let mut both = a.clone();
    both.extend(b.iter().copied());
    let outer = p.region(Kind::Table, both);
    let right = p.region(Kind::Paragraph, b);
    let left = p.region(Kind::Paragraph, a);
    p.regions[right.0 as usize].parent = Some(outer);
    p.regions[left.0 as usize].parent = Some(outer);
    let bottom = p.para(50.0, 640.0, 3, 80);
    assert_eq!(p.ask(), [top, outer, left, right, bottom]);
}

#[test]
fn odd_input_keeps_every_region() {
    let p = Page::new();
    assert!(order(&p.page, &[]).is_empty());
    // Many regions on one spot, some with no extent and some with no number at all.
    let mut p = Page::new();
    for k in 0..200 {
        let b = match k % 4 {
            0 => Bounds {
                x0: 10.0,
                y0: 10.0,
                x1: 10.0,
                y1: 10.0,
            },
            1 => Bounds {
                x0: f32::NAN,
                y0: 5.0,
                x1: f32::INFINITY,
                y1: f32::NAN,
            },
            2 => Bounds {
                x0: 50.0,
                y0: 700.0 - k as f32,
                x1: 20.0,
                y1: 690.0 - k as f32,
            },
            _ => Bounds {
                x0: 0.0,
                y0: 0.0,
                x1: 600.0,
                y1: 800.0,
            },
        };
        p.region_in(Kind::Spatial, Vec::new(), b);
    }
    p.ask();
}

#[test]
fn a_deep_page_keeps_its_columns() {
    // A hundred rows of two regions each, the white space between them all the same and their
    // gutters at two places in turn, so that no gutter runs down the rows; under them two
    // columns with a gutter of their own: deeper than the cut goes when it cuts off one row at
    // a time.
    let mut p = Page::new();
    let mut want = Vec::new();
    for k in 0..100 {
        let y = 790.0 - 2.5 * k as f32;
        let split = if k % 2 == 0 { 240.0 } else { 340.0 };
        let l = p.lines(50.0, y, 1, ((split - 50.0) * 2.0) as usize, 1.0);
        want.push(p.region(Kind::Paragraph, l));
        let r = p.lines(split + 20.0, y, 1, ((530.0 - split) * 2.0) as usize, 1.0);
        want.push(p.region(Kind::Paragraph, r));
    }
    let y = 790.0 - 2.5 * 100.0;
    let l = p.lines(50.0, y, 1, 100, 1.0);
    let l1 = p.region(Kind::Paragraph, l);
    let l = p.lines(50.0, y - 2.5, 1, 100, 1.0);
    let l2 = p.region(Kind::Paragraph, l);
    let l = p.lines(200.0, y, 3, 500, 1.0);
    let r = p.region(Kind::Paragraph, l);
    want.extend([l1, l2, r]);
    assert_eq!(p.ask(), want);
}

/// The work steps of ordering two columns of `n` one-line paragraphs each, a title over them.
fn work_of(n: usize) -> Option<u64> {
    let mut p = Page::new();
    let t = p.lines(60.0, 795.0, 1, 600, 1.0);
    p.region(Kind::Heading, t);
    for x in [50.0, 320.0] {
        for k in 0..n {
            let y = 790.0 - 790.0 * k as f32 / n as f32;
            let l = p.lines(x, y, 1, 400, 0.1);
            p.region(Kind::Paragraph, l);
        }
    }
    let before = work_steps()?;
    let got = order(&p.page, &p.regions);
    let after = work_steps()?;
    assert_eq!(got.len(), 2 * n + 1);
    Some(after - before)
}

#[test]
fn work_grows_with_the_regions_not_their_square() {
    let (Some(small), Some(large)) = (work_of(1000), work_of(4000)) else {
        // A release build does not count work.
        return;
    };
    assert!(
        large < 8 * small,
        "{small} steps for 1000, {large} for 4000"
    );
}

/// The order and the work steps of ordering, in the source order, `n` paragraphs of two lines
/// across the page with a picture under each: every line shares every picture's width and no
/// region holds a picture, the most a picture's place looks at. `per` is the work for placing
/// pictures by their column (see `with_place_work`).
fn order_of_pictures(n: usize, per: Option<u64>) -> (Vec<RegionId>, Option<u64>) {
    let mut p = Page::new();
    let gap = 780.0 / n as f32;
    let mut source = Vec::new();
    for k in 0..n {
        let y = 790.0 - gap * k as f32;
        let l = p.lines(50.0, y, 2, 20000, 0.02);
        source.push(p.region(Kind::Paragraph, l));
        let pic = p.picture(100.0, y - 0.75 * gap, 300.0, y - 0.45 * gap);
        p.region(Kind::Figure, vec![pic]);
    }
    let before = work_steps();
    let got = match per {
        Some(per) => with_place_work(per, || order_by_source(&p.page, &p.regions, &source)),
        None => order_by_source(&p.page, &p.regions, &source),
    };
    let after = work_steps();
    // Paragraph k, then the picture under it.
    let want: Vec<RegionId> = (0..2 * n as u32).map(RegionId).collect();
    assert_eq!(got, want, "{n} paragraphs");
    (got, before.zip(after).map(|(b, a)| a - b))
}

/// Task 22 fix round 4: placing the pictures of a page in the source order grows with its
/// pictures and lines, not with their product. Fix round 5: the pictures placed past the
/// budget (by their height) stand where the full rule puts them.
#[test]
fn placing_pictures_grows_with_the_page_not_pictures_times_lines() {
    let (bounded, small) = order_of_pictures(1000, None);
    let (full, all) = order_of_pictures(1000, Some(u64::MAX));
    assert_eq!(bounded, full);
    let (_, large) = order_of_pictures(4000, None);
    let (Some(small), Some(all), Some(large)) = (small, all, large) else {
        // A release build does not count work.
        return;
    };
    // The budget is reached: some pictures are placed by their height.
    assert!(small < all, "{small} steps bounded, {all} without a bound");
    assert!(
        large < 8 * small,
        "{small} steps for 1000, {large} for 4000"
    );
}

/// The order and the work steps of ordering, in the source order, `n` one-line regions side by
/// side in one row level with a picture's top (within one text size under it), right of the
/// picture, then a paragraph under the picture in its width. Each region of the row is level
/// with the top and looks along the rest of the row for a region of its width that reaches
/// down; none does, so the picture stands by its height, before the paragraph. `per` is the
/// work for placing pictures by their column (see `with_place_work`).
fn order_of_a_row(n: usize, per: u64) -> (Vec<RegionId>, Option<u64>) {
    let mut p = Page::new();
    let pic = p.picture(50.0, 600.0, 150.0, 700.0);
    let figure = p.region(Kind::Figure, vec![pic]);
    let width = 390.0 / n as f32;
    let mut source = Vec::new();
    for k in 0..n {
        let x0 = 200.0 + width * k as f32;
        let l = p.lines(x0, 692.0, 1, 1, BODY);
        let bounds = Bounds {
            x0,
            y0: 690.0,
            x1: x0 + width,
            y1: 698.0,
        };
        source.push(p.region_in(Kind::Paragraph, l, bounds));
    }
    let under = p.para(50.0, 580.0, 4, 20);
    source.push(under);
    let before = work_steps();
    let got = with_place_work(per, || order_by_source(&p.page, &p.regions, &source));
    let after = work_steps();
    let mut want = source[..n].to_vec();
    want.extend([figure, under]);
    assert_eq!(got, want, "{n} regions in the row");
    (got, before.zip(after).map(|(b, a)| a - b))
}

/// Task 22 fix round 5: the regions level with a picture's top each look along the others of
/// their row; that look is in the budget too, so the work grows with the row, not with its
/// square, and the picture past the budget stands where the full rule puts it. The work for
/// each region is set small so that a row of a thousand regions reaches it; the bound does not
/// depend on that number.
#[test]
fn a_row_level_with_a_picture_top_costs_about_its_length() {
    let (bounded, small) = order_of_a_row(1000, 16);
    let (full, all) = order_of_a_row(1000, u64::MAX);
    assert_eq!(bounded, full);
    let (_, large) = order_of_a_row(4000, 16);
    let (Some(small), Some(all), Some(large)) = (small, all, large) else {
        // A release build does not count work.
        return;
    };
    assert!(small < all, "{small} steps bounded, {all} without a bound");
    assert!(
        large < 8 * small,
        "{small} steps for 1000, {large} for 4000"
    );
}

/// Task 22 fix round 5: a picture set into the top of a paragraph (its first lines beside the
/// picture, the rest under it) stands before that paragraph, whether the paragraph's box
/// starts a little above the picture's top or a little under it: a region of the picture's
/// own width level with its top counts as one that starts above it.
#[test]
fn a_picture_set_into_the_top_of_a_paragraph_stands_before_it() {
    for box_top in [700.5, 699.5, 695.0] {
        let mut p = Page::new();
        let pic = p.picture(50.0, 640.0, 150.0, 700.0);
        let figure = p.region(Kind::Figure, vec![pic]);
        let mut prims = p.lines(160.0, 690.0, 5, 70, BODY);
        prims.extend(p.lines(50.0, 625.0, 4, 90, BODY));
        let bounds = Bounds {
            x0: 50.0,
            y0: 585.0,
            x1: 500.0,
            y1: box_top,
        };
        let para = p.region_in(Kind::Paragraph, prims, bounds);
        let regions = p.scrambled();
        let got = order_by_source(&p.page, &regions, &[para]);
        assert_once(&regions, &got);
        assert_eq!(got, [figure, para], "box top {box_top}");
    }
}

/// Task 22: in the source order, the regions read as the source has them, even across the
/// columns a gutter would read down; a figure (no place in the source) stands before the first
/// of them whose top is lower than its middle; furniture stays first and last.
#[test]
fn source_order_holds_and_places_what_it_lacks() {
    let mut p = Page::new();
    let l1 = p.para(50.0, 760.0, 4, 44);
    let pic = p.picture(60.0, 600.0, 250.0, 700.0);
    let figure = p.region(Kind::Figure, vec![pic]);
    let l2 = p.para(50.0, 580.0, 4, 44);
    let r = p.para(320.0, 760.0, 16, 44);
    let f = p.lines(50.0, 40.0, 1, 20, BODY);
    let foot = p.region(Kind::FooterBand, f);
    // The source reads the right column between the two left paragraphs.
    let source = [foot, l1, r, l2];
    let regions = p.scrambled();
    let got = order_by_source(&p.page, &regions, &source);
    assert_once(&regions, &got);
    assert_eq!(got, [l1, r, figure, l2, foot]);
}

/// Task 22 fix round 1: the mirror. A figure without text in the right column of a page the
/// source reads column by column stays in the right column, between its paragraphs; it does
/// not go into the left column at its height.
#[test]
fn a_loose_figure_keeps_to_its_column() {
    let mut p = Page::new();
    let l1 = p.para(50.0, 760.0, 4, 44);
    let l2 = p.para(50.0, 580.0, 4, 44);
    let r1 = p.para(320.0, 760.0, 4, 44);
    let pic = p.picture(330.0, 600.0, 520.0, 700.0);
    let figure = p.region(Kind::Figure, vec![pic]);
    let r2 = p.para(320.0, 580.0, 4, 44);
    let source = [l1, l2, r1, r2];
    let regions = p.scrambled();
    let got = order_by_source(&p.page, &regions, &source);
    assert_once(&regions, &got);
    assert_eq!(got, [l1, l2, r1, figure, r2]);
}

/// Task 22 fix round 2: a figure at the top of the right column (nothing of its width over it,
/// the left column's text over it) stands before the right column's text, not in the left
/// column at its height; one at the foot of the right column (its column's text over it,
/// nothing of its width under it) stands after the right column's text, though a left
/// paragraph starts below its middle.
#[test]
fn a_loose_figure_at_the_top_or_foot_of_its_column() {
    let mut p = Page::new();
    let l1 = p.para(50.0, 760.0, 4, 44);
    let l2 = p.para(50.0, 580.0, 4, 44);
    let pic = p.picture(330.0, 680.0, 520.0, 760.0);
    let figure = p.region(Kind::Figure, vec![pic]);
    let r1 = p.para(320.0, 660.0, 4, 44);
    let source = [l1, l2, r1];
    let regions = p.scrambled();
    let got = order_by_source(&p.page, &regions, &source);
    assert_once(&regions, &got);
    assert_eq!(got, [l1, l2, figure, r1]);

    let mut p = Page::new();
    let l1 = p.para(50.0, 760.0, 4, 44);
    let l2 = p.para(50.0, 580.0, 4, 44);
    let l3 = p.para(50.0, 410.0, 4, 44);
    let r1 = p.para(320.0, 760.0, 4, 44);
    let r2 = p.para(320.0, 580.0, 4, 44);
    let pic = p.picture(330.0, 380.0, 520.0, 480.0);
    let figure = p.region(Kind::Figure, vec![pic]);
    let source = [l1, l2, l3, r1, r2];
    let regions = p.scrambled();
    let got = order_by_source(&p.page, &regions, &source);
    assert_once(&regions, &got);
    assert_eq!(got, [l1, l2, l3, r1, r2, figure]);
}

/// Task 22 fix round 3: a figure at the top of the right column under a title across both
/// columns stands before the right column's text: the title over it is no reading of its
/// column, and the source reads the left column, which stands across the figure's top, first.
#[test]
fn a_loose_figure_under_a_band_at_the_top_of_its_column() {
    let mut p = Page::new();
    let t = p.lines(50.0, 790.0, 1, 100, BODY);
    let title = p.region(Kind::Heading, t);
    let l1 = p.para(50.0, 760.0, 4, 44);
    let l2 = p.para(50.0, 580.0, 4, 44);
    let pic = p.picture(330.0, 680.0, 520.0, 760.0);
    let figure = p.region(Kind::Figure, vec![pic]);
    let r1 = p.para(320.0, 660.0, 4, 44);
    let source = [title, l1, l2, r1];
    let regions = p.scrambled();
    let got = order_by_source(&p.page, &regions, &source);
    assert_once(&regions, &got);
    assert_eq!(got, [title, l1, l2, figure, r1]);
}

/// Task 22 fix round 4: a figure set flush with the top of the right column stands before the
/// right column's text, with a band over the columns and without one. Columns start level: a
/// picture is set at its frame's edge, a line box starts at its ascender, so the figure's top
/// stands up to a line above the top of the left column's first line. The order does not
/// flip on a few points of that height.
#[test]
fn a_figure_flush_with_the_column_top_stays_in_its_column() {
    for band in [true, false] {
        for top in [769.0, 770.0, 772.0, 775.0, 779.0] {
            let mut p = Page::new();
            let title = band.then(|| {
                let t = p.lines(50.0, 790.0, 1, 100, BODY);
                p.region(Kind::Heading, t)
            });
            let l1 = p.para(50.0, 760.0, 4, 44);
            let l2 = p.para(50.0, 580.0, 4, 44);
            let pic = p.picture(330.0, 680.0, 520.0, top);
            let figure = p.region(Kind::Figure, vec![pic]);
            let r1 = p.para(320.0, 660.0, 4, 44);
            let mut source: Vec<RegionId> = title.into_iter().collect();
            source.extend([l1, l2, r1]);
            let regions = p.scrambled();
            let got = order_by_source(&p.page, &regions, &source);
            assert_once(&regions, &got);
            let mut want: Vec<RegionId> = title.into_iter().collect();
            want.extend([l1, l2, figure, r1]);
            assert_eq!(got, want, "band {band}, figure top {top}");
        }
    }
}

/// Task 22 fix round 4: the same when the left column opens with a one-line heading level with
/// the figure's top: the heading is no label beside the figure, because the source reads on down
/// its column (its first paragraph) before it reaches the figure's column.
#[test]
fn a_figure_flush_with_a_column_that_opens_with_a_heading() {
    for band in [true, false] {
        for top in [770.0, 775.0, 779.0] {
            let mut p = Page::new();
            let title = band.then(|| {
                let t = p.lines(50.0, 790.0, 1, 100, BODY);
                p.region(Kind::Heading, t)
            });
            let h = p.lines(50.0, 765.0, 1, 30, BODY);
            let head = p.region(Kind::Heading, h);
            let l1 = p.para(50.0, 745.0, 4, 44);
            let l2 = p.para(50.0, 580.0, 4, 44);
            let pic = p.picture(330.0, 680.0, 520.0, top);
            let figure = p.region(Kind::Figure, vec![pic]);
            let r1 = p.para(320.0, 660.0, 4, 44);
            let mut source: Vec<RegionId> = title.into_iter().collect();
            source.extend([head, l1, l2, r1]);
            let regions = p.scrambled();
            let got = order_by_source(&p.page, &regions, &source);
            assert_once(&regions, &got);
            let mut want: Vec<RegionId> = title.into_iter().collect();
            want.extend([head, l1, l2, figure, r1]);
            assert_eq!(got, want, "band {band}, figure top {top}");
        }
    }
}

/// Task 22 fix round 4: a code at the right of a page head, its top level with a one-line
/// heading on the left. The source reads the head band by band: the left block under the
/// heading ends above the paragraph across the page under the code, so the heading starts no
/// column read down past the code, and the code keeps its place by its height, at the head.
#[test]
fn a_code_level_with_a_heading_of_a_page_head_stays_at_the_head() {
    let mut p = Page::new();
    let h = p.lines(50.0, 798.0, 1, 30, BODY);
    let head = p.region(Kind::Heading, h);
    let first = p.para(50.0, 770.0, 2, 40);
    let second = p.para(50.0, 740.0, 3, 60);
    let pic = p.picture(479.0, 754.0, 539.0, 808.0);
    let code = p.region(Kind::Code, vec![pic]);
    let across = p.para(50.0, 690.0, 4, 100);
    let source = [head, first, second, across];
    let regions = p.scrambled();
    let got = order_by_source(&p.page, &regions, &source);
    assert_once(&regions, &got);
    assert_eq!(got, [head, code, first, second, across]);
}

/// Task 22 fix round 4: a code at the foot of its column (a link over it, nothing of its width
/// under it), beside a column of links whose block starts a few points under the code's top. A
/// column level with a picture's top says a column top only where the picture's column goes
/// on under it; here the code keeps its place among the links beside it, before the first one
/// under its middle, not after the whole page.
#[test]
fn a_code_at_the_foot_of_its_column_stays_beside_its_links() {
    let mut p = Page::new();
    let r = p.lines(466.0, 640.0, 1, 20, BODY);
    let over = p.region(Kind::Paragraph, r);
    let h = p.lines(322.0, 362.1, 1, 20, 9.5);
    let head = p.region(Kind::Heading, h);
    let a = p.lines(331.0, 348.0, 3, 20, 9.0);
    let first = p.region(Kind::List, a);
    let b = p.lines(331.0, 312.0, 2, 20, 9.0);
    let second = p.region(Kind::List, b);
    let pic = p.picture(485.0, 309.0, 537.0, 362.0);
    let code = p.region(Kind::Code, vec![pic]);
    let source = [over, head, first, second];
    let regions = p.scrambled();
    let got = order_by_source(&p.page, &regions, &source);
    assert_once(&regions, &got);
    assert_eq!(got, [over, head, first, code, second]);
}

/// Task 22 fix round 3: a figure across both columns between two bands of them. A source read
/// band by band gives it its place between the bands. A source read column by column reads
/// across it at the left column's crossing: it stands there, after the left column's upper
/// paragraph (the first place a reader of the page meets it), not after the right one.
#[test]
fn a_figure_across_the_columns_between_bands() {
    let mut p = Page::new();
    let l1 = p.para(50.0, 760.0, 4, 44);
    let r1 = p.para(320.0, 760.0, 4, 44);
    let pic = p.picture(50.0, 620.0, 540.0, 700.0);
    let figure = p.region(Kind::Figure, vec![pic]);
    let l2 = p.para(50.0, 580.0, 4, 44);
    let r2 = p.para(320.0, 580.0, 4, 44);
    let regions = p.scrambled();
    let got = order_by_source(&p.page, &regions, &[l1, r1, l2, r2]);
    assert_once(&regions, &got);
    assert_eq!(got, [l1, r1, figure, l2, r2]);
    let got = order_by_source(&p.page, &regions, &[l1, l2, r1, r2]);
    assert_eq!(got, [l1, figure, l2, r1, r2]);
}

/// Task 22 fix round 3: a logo at the top left of a page head, beside a region the segmenter
/// made across the gutter (a short left paragraph low on the page and the right address
/// beside the logo). That region's box reaches over and under the logo, but its content in
/// the logo's width is the short paragraph only: it neither holds the logo nor stands over
/// it. The logo stands by its column: after the title over it, before the address under it.
#[test]
fn a_region_across_the_gutter_holds_no_picture_by_its_box() {
    let mut p = Page::new();
    let t = p.lines(18.0, 790.0, 1, 30, BODY);
    let title = p.region(Kind::Heading, t);
    let pic = p.picture(18.0, 720.0, 122.0, 774.0);
    let logo = p.region(Kind::Figure, vec![pic]);
    let a = p.lines(18.0, 700.0, 3, 24, BODY);
    let address = p.region(Kind::Paragraph, a);
    let mut merged = p.lines(18.0, 560.0, 2, 30, BODY);
    merged.extend(p.lines(300.0, 770.0, 5, 20, BODY));
    let joined = p.region(Kind::Paragraph, merged);
    let source = [title, address, joined];
    let regions = p.scrambled();
    let got = order_by_source(&p.page, &regions, &source);
    assert_once(&regions, &got);
    assert_eq!(got, [title, logo, address, joined]);
}
