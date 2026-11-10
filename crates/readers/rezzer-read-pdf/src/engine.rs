// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The page as input of the region engine (`layoutregions`): the runs, rules, boxes, pictures
//! and codes the reader found, as primitives with the ids the reader keeps for them.

#[cfg(test)]
use layoutregions::segment::drawn::{DrawnStructure, Grids};
use layoutregions::{
    Bounds, BoxIn, CodeIn, FontKind, GapIn, PageIn, PictureIn, PrimId, RuleIn, Shift, TextRun,
};
use layoutregions::{GivenRegion, Kind};

use crate::layout::{Drawn, Run};
use crate::order::Turn;
use crate::tables::{Rule, RuleKind};
use crate::text::PageItems;

/// The primitives of one page. `runs` are the runs of the page's flow in order (after the
/// runs a tag tree's ActualText drops): `TextRun` *i* is `runs[i]`, so `PrimId::Text(i)` is the
/// run's id in the flow (`Flow::ids`), which moves with it while detectors take runs out or
/// read them in another order. `size` is the frame the runs are in, turned if the page was
/// turned.
pub(crate) fn page_in(
    items: &PageItems,
    runs: &[Run],
    drawn: &Drawn<'_>,
    size: [f32; 2],
) -> PageIn {
    let texts = runs.iter().map(text_run).collect();
    let rules = drawn.rules.iter().map(rule_in).collect();
    let boxes = drawn
        .boxes
        .iter()
        .map(|b| box_in(b, false))
        .chain(drawn.widgets.iter().map(|b| box_in(b, true)))
        .collect();
    let pictures = drawn
        .images
        .iter()
        .map(|&b| PictureIn { bounds: bounds(b) })
        .collect();
    // The code boxes are in the page's own frame, the pictures' (see `PageItems::codes`).
    let (w, h) = match items.turn {
        Turn::Up | Turn::Down => (size[1], size[0]),
        Turn::None | Turn::Half => (size[0], size[1]),
    };
    let codes = items
        .codes
        .iter()
        .map(|c| {
            let b = if items.turned {
                items.turn.place_box(c.b, w, h)
            } else {
                c.b
            };
            CodeIn { bounds: bounds(b) }
        })
        .collect();
    PageIn {
        page: items.page,
        size,
        turn: match items.turn {
            Turn::None => 0,
            Turn::Up => 1,
            Turn::Half => 2,
            Turn::Down => 3,
        },
        texts,
        rules,
        boxes,
        pictures,
        codes,
        given: Vec::new(),
    }
}

/// The drawn structure of a page that draws nothing, for tests: a `Drawn` with no rules. A
/// test with rules makes the structure from them (`regions::structure_in_frame`), so the two
/// never disagree.
#[cfg(test)]
pub(crate) const NO_STRUCTURE: DrawnStructure = DrawnStructure {
    grids: Ok(Grids {
        hs: Vec::new(),
        vs: Vec::new(),
        groups: Vec::new(),
    }),
    boxes: Vec::new(),
    boxes_note: None,
};

pub(crate) fn text_run(r: &Run) -> TextRun {
    TextRun {
        text: r.text.clone(),
        x: r.x,
        y: r.y,
        w: r.w,
        h: r.h,
        size: r.size,
        font: r.family,
        font_kind: FontKind::Unknown,
        bold: r.bold,
        italic: r.italic,
        color: None,
        shift: if r.note { Shift::Raised } else { Shift::Base },
        line: r.line,
        off_axis: r.off_axis,
        gaps: r
            .gaps
            .iter()
            .map(|g| GapIn {
                words: g.words,
                from: g.from,
                to: g.to,
            })
            .collect(),
    }
}

/// A drawn rule as the engine takes it.
pub(crate) fn rule_in(r: &Rule) -> RuleIn {
    RuleIn {
        x0: r.x0,
        y0: r.y0,
        x1: r.x1,
        y1: r.y1,
        thick: 0.0,
        dashed: false,
        rect: r.kind == RuleKind::Rect,
    }
}

pub(crate) fn bounds(b: [f32; 4]) -> Bounds {
    Bounds {
        x0: b[0],
        y0: b[1],
        x1: b[2],
        y1: b[3],
    }
}

pub(crate) fn box_in(b: &[f32; 4], widget: bool) -> BoxIn {
    BoxIn {
        bounds: bounds(*b),
        filled: false,
        stroked: false,
        widget,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Gaps;

    fn run(text: &str, x: f32, y: f32, line: u32) -> Run {
        Run {
            page: 1,
            text: text.into(),
            x,
            y,
            w: 40.0,
            h: 10.0,
            size: 10.0,
            bold: false,
            italic: false,
            family: 0,
            mcid: None,
            uri: None,
            line,
            off_axis: false,
            note: false,
            gaps: Gaps::default(),
        }
    }

    const NOTHING: Drawn<'static> = Drawn {
        rules: &[],
        structure: &NO_STRUCTURE,
        images: &[],
        widgets: &[],
        boxes: &[],
        items: None,
        lines_unsearched: false,
        width: 200.0,
        body_size: 10.0,
    };

    #[test]
    fn runs_keep_their_index_and_geometry() {
        let mut b = run("Bold", 70.0, 600.0, 1);
        b.bold = true;
        b.size = 12.0;
        b.h = 12.0;
        let runs = [run("One", 10.0, 700.0, 0), b, run("Three", 30.0, 500.0, 2)];
        let page = page_in(&PageItems::default(), &runs, &NOTHING, [200.0, 800.0]);
        assert_eq!(page.texts.len(), 3);
        let t = &page.texts[1];
        assert_eq!(t.text, "Bold");
        assert_eq!(
            (t.x, t.y, t.w, t.h, t.size),
            (70.0, 600.0, 40.0, 12.0, 12.0)
        );
        assert!(t.bold);
        assert_eq!(t.line, 1);
        assert_eq!(page.texts[2].line, 2);
        assert_eq!(page.size, [200.0, 800.0]);
        assert_eq!(page.turn, 0);
    }

    #[test]
    fn rules_and_rects_map_with_their_kind() {
        let rules = [
            Rule {
                x0: 10.0,
                y0: 50.0,
                x1: 90.0,
                y1: 50.0,
                kind: RuleKind::Line,
            },
            Rule {
                x0: 10.0,
                y0: 60.0,
                x1: 90.0,
                y1: 80.0,
                kind: RuleKind::Rect,
            },
        ];
        let structure = crate::regions::structure_in_frame(&rules, &[]);
        let drawn = Drawn {
            rules: &rules,
            structure: &structure,
            images: &[[1.0, 2.0, 3.0, 4.0]],
            ..NOTHING
        };
        let page = page_in(&PageItems::default(), &[], &drawn, [200.0, 800.0]);
        assert_eq!(page.rules.len(), 2);
        assert!(!page.rules[0].rect);
        assert!(page.rules[1].rect);
        assert_eq!((page.rules[0].thick, page.rules[1].thick), (0.0, 0.0));
        assert!(!page.rules[0].dashed);
        assert_eq!((page.rules[1].x0, page.rules[1].y1), (10.0, 80.0));
        assert_eq!(page.pictures.len(), 1);
        assert_eq!(page.pictures[0].bounds.x1, 3.0);
    }

    /// The structure of the test constants is the one a page without rules or boxes has.
    #[test]
    fn no_structure_is_that_of_no_rules() {
        assert_eq!(crate::regions::structure_in_frame(&[], &[]), NO_STRUCTURE);
        assert!(NOTHING.rules.is_empty() && NOTHING.boxes.is_empty());
    }

    #[test]
    fn widgets_are_boxes_marked_widget() {
        let boxes = [[0.0, 0.0, 10.0, 10.0]];
        let structure = crate::regions::structure_in_frame(&[], &boxes);
        let drawn = Drawn {
            structure: &structure,
            boxes: &boxes,
            widgets: &[[20.0, 20.0, 60.0, 34.0]],
            ..NOTHING
        };
        let page = page_in(&PageItems::default(), &[], &drawn, [200.0, 800.0]);
        let marked: Vec<bool> = page.boxes.iter().map(|b| b.widget).collect();
        assert_eq!(marked, [false, true]);
        assert_eq!(page.boxes[1].bounds.y1, 34.0);
    }

    #[test]
    fn a_footnote_reference_is_raised() {
        let mut note = run("1", 50.0, 706.0, 0);
        note.note = true;
        let runs = [run("Text", 10.0, 700.0, 0), note];
        let page = page_in(&PageItems::default(), &runs, &NOTHING, [200.0, 800.0]);
        assert_eq!(page.texts[0].shift, Shift::Base);
        assert_eq!(page.texts[1].shift, Shift::Raised);
    }

    #[test]
    fn turned_page_keeps_the_turned_frame() {
        // A page 200 wide and 800 high, turned a quarter: 800 wide and 200 high.
        let items = PageItems {
            turn: Turn::Up,
            turned: true,
            // In the page's own frame, 200 wide and 800 high: above the height of the turned
            // frame, so only the turn puts it inside.
            codes: vec![crate::codes::VectorCode {
                b: [10.0, 500.0, 50.0, 560.0],
                module: 2.0,
                id: 1,
                finders: 0,
                font: false,
                texts: Vec::new(),
            }],
            ..PageItems::default()
        };
        let runs = [run("A", 10.0, 100.0, 0), run("B", 700.0, 20.0, 1)];
        let page = page_in(&items, &runs, &NOTHING, [800.0, 200.0]);
        assert_eq!(page.turn, 1);
        assert_eq!(page.size, [800.0, 200.0]);
        for t in &page.texts {
            assert!((0.0..=800.0).contains(&t.x) && (0.0..=200.0).contains(&t.y));
        }
        // The code is placed into the turned frame like the pictures.
        let c = page.codes[0].bounds;
        assert!(c.x0 >= 0.0 && c.x1 <= 800.0 && c.y0 >= 0.0 && c.y1 <= 200.0);
    }
}

/// The role of the given region of a form field (see [`given_regions`]).
pub(crate) const FIELD_ROLE: &str = "form-field";

/// The regions a page states itself, as the region engine takes them: every block of the tag
/// tree the layout reads (`slots`, one per run of `page.texts`, in their order) and every
/// form field box. A heading element is a `Heading` with the level its role gives, a
/// paragraph element a `Paragraph` (a caption with the role `caption`), a note a
/// `FootnoteLegend`, a list item a `List` region of its own whose `group` is the order of the
/// list's first item, a table (its cells) a `Table` when `table_kept` says its table is read
/// as one (the cells of a tag table that does not hold are free text); each holds the runs of
/// its element. A form field is the form's layer: a `Paragraph` of the role [`FIELD_ROLE`] of
/// its box and the runs whose centre lies in it that no tag element holds (the runs sorted by
/// their centre once), so no rule of the page reads pairs or tables over it. Untagged runs
/// belong to none. In the order of their first run; the fields after the tag elements.
pub(crate) fn given_regions(
    page: &PageIn,
    slots: &[crate::layout::Slot],
    table_kept: &dyn Fn(usize) -> bool,
) -> Vec<GivenRegion> {
    use crate::layout::Slot;
    use std::collections::HashMap;
    let mut at: HashMap<(u8, usize), usize> = HashMap::new();
    let mut lists: HashMap<usize, u32> = HashMap::new();
    let mut out: Vec<GivenRegion> = Vec::new();
    let mut held = vec![false; page.texts.len()];
    for (i, slot) in slots.iter().enumerate().take(page.texts.len()) {
        let (key, kind, level, role, list) = match *slot {
            Slot::Heading(level, e) => ((0, e), Kind::Heading, Some(level), None, None),
            Slot::Para(e, Some("footnote")) => ((1, e), Kind::FootnoteLegend, None, None, None),
            Slot::Para(e, Some("caption")) => {
                ((1, e), Kind::Paragraph, None, Some("caption"), None)
            }
            Slot::Para(e, _) => ((1, e), Kind::Paragraph, None, None, None),
            Slot::Item { list, item } => ((2, item), Kind::List, None, None, Some(list)),
            Slot::Cell { table, .. } if table_kept(table) => {
                ((3, table), Kind::Table, None, None, None)
            }
            Slot::Cell { .. } | Slot::Free => continue,
        };
        held[i] = true;
        let k = *at.entry(key).or_insert_with(|| {
            let order = out.len() as u32;
            let group = list.map(|l| *lists.entry(l).or_insert(order));
            out.push(GivenRegion {
                kind,
                prims: Vec::new(),
                order,
                level,
                role,
                group,
            });
            out.len() - 1
        });
        out[k].prims.push(PrimId::Text(i as u32));
    }
    // The free runs by the x of their centre, for the field boxes.
    let mut by_x: Vec<(f32, f32, u32)> = page
        .texts
        .iter()
        .enumerate()
        .filter(|(i, _)| !held[*i])
        .map(|(i, r)| (r.x + r.w / 2.0, r.y + r.h / 2.0, i as u32))
        .collect();
    by_x.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (b, field) in page.boxes.iter().enumerate().filter(|(_, b)| b.widget) {
        let f = field.bounds;
        let lo = by_x.partition_point(|r| r.0 < f.x0);
        let hi = by_x.partition_point(|r| r.0 <= f.x1);
        let mut inside: Vec<u32> = by_x[lo..hi]
            .iter()
            .filter(|r| r.1 >= f.y0 && r.1 <= f.y1 && !held[r.2 as usize])
            .map(|r| r.2)
            .collect();
        inside.sort_unstable();
        let mut prims = vec![PrimId::Box(b as u32)];
        for i in inside {
            held[i as usize] = true;
            prims.push(PrimId::Text(i));
        }
        out.push(GivenRegion {
            kind: Kind::Paragraph,
            prims,
            order: out.len() as u32,
            level: None,
            role: Some(FIELD_ROLE),
            group: None,
        });
    }
    out
}

#[cfg(test)]
mod given_tests {
    use super::*;
    use crate::layout::Slot;

    fn text(t: &str, x: f32, y: f32) -> TextRun {
        TextRun {
            text: t.into(),
            x,
            y,
            w: 40.0,
            h: 8.0,
            size: 8.0,
            font: 0,
            font_kind: FontKind::Proportional,
            bold: false,
            italic: false,
            color: None,
            shift: Shift::Base,
            line: 0,
            off_axis: false,
            gaps: Vec::new(),
        }
    }

    /// Task 18b (ruling Q1): every tag block the layout reads is a region the page states, a
    /// heading with its level, a list with its items, a table with its cells; a form field box
    /// with the runs in it that no tag holds; untagged runs are in none.
    #[test]
    fn tag_blocks_and_form_fields_are_given_regions() {
        let mut page = PageIn {
            page: 1,
            size: [600.0, 800.0],
            turn: 0,
            texts: (0..7)
                .map(|k| text("Muster", 50.0, 700.0 - 20.0 * k as f32))
                .collect(),
            rules: Vec::new(),
            boxes: Vec::new(),
            pictures: Vec::new(),
            codes: Vec::new(),
            given: Vec::new(),
        };
        // A field box over the footnote's run (a tag holds it) and a free run.
        page.boxes.push(box_in(&[45.0, 575.0, 200.0, 612.0], true));
        let slots = [
            Slot::Heading(2, 10),
            Slot::Para(11, Some("caption")),
            Slot::Item { list: 12, item: 13 },
            Slot::Item { list: 12, item: 14 },
            Slot::Cell {
                table: 15,
                cell: 16,
            },
            Slot::Para(17, Some("footnote")),
            Slot::Free,
        ];
        let given = given_regions(&page, &slots, &|_| true);
        type Got = (
            Kind,
            Option<u8>,
            Option<&'static str>,
            Option<u32>,
            Vec<PrimId>,
        );
        let got: Vec<Got> = given
            .iter()
            .map(|g| (g.kind, g.level, g.role, g.group, g.prims.clone()))
            .collect();
        let field = vec![PrimId::Box(0), PrimId::Text(6)];
        assert_eq!(
            got,
            [
                (Kind::Heading, Some(2), None, None, vec![PrimId::Text(0)]),
                (
                    Kind::Paragraph,
                    None,
                    Some("caption"),
                    None,
                    vec![PrimId::Text(1)]
                ),
                (Kind::List, None, None, Some(2), vec![PrimId::Text(2)]),
                (Kind::List, None, None, Some(2), vec![PrimId::Text(3)]),
                (Kind::Table, None, None, None, vec![PrimId::Text(4)]),
                (
                    Kind::FootnoteLegend,
                    None,
                    None,
                    None,
                    vec![PrimId::Text(5)]
                ),
                (Kind::Paragraph, None, Some(FIELD_ROLE), None, field),
            ]
        );
        assert!(given.iter().enumerate().all(|(k, g)| g.order == k as u32));
    }
}
