// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::time::Instant;

use rezzer_derez::grid::Pos;
use rezzer_derez::plan::{MARGIN, MAX_PLANS, Plan, Shape, ShapeKind, fit, plans, rounds};
use rezzer_model::{
    Anchor, Block, Cell, Certainty, Confidence, Detection, Document, FormatId, Formula, Inline,
    LayoutKind, SectionKind, SourceInfo, TableLayout, TaskState,
};

fn doc(body: Vec<Block>) -> Document {
    let mut d = Document::new(SourceInfo {
        path: None,
        name: "invented.xlsx".into(),
        sha256: "00".into(),
        size: 1,
        format: FormatId::Csv,
        detection: Detection {
            confidence: Confidence::Certain,
            evidence: "test".into(),
        },
        modified: None,
        created: None,
        downloaded_from: None,
        file_tags: Vec::new(),
        encoding: None,
        reader: None,
    });
    d.body = body;
    d
}

fn text(s: &str) -> Vec<Inline> {
    vec![Inline::plain(s)]
}

fn heading(depth: u8, s: &str) -> Block {
    Block::Heading {
        level: depth,
        inlines: text(s),
        anchor: None,
    }
}

fn para(s: &str) -> Block {
    Block::paragraph(text(s))
}

fn table(rows: usize, cols: usize, range: Option<&str>) -> Block {
    let row = |r: usize| {
        (0..cols)
            .map(|c| Cell::new(text(&format!("r{r}c{c}"))))
            .collect()
    };
    Block::Table {
        caption: None,
        header_rows: 1,
        role: None,
        rows: (0..rows).map(row).collect(),
        alignments: Vec::new(),
        anchor: None,
        layout: range.map(|r| {
            Box::new(TableLayout {
                kind: LayoutKind::Table,
                key: "k".into(),
                range: r.into(),
                data: None,
                header_rows: Vec::new(),
                header_paths: Vec::new(),
                label_cols: 0,
                total_columns: Vec::new(),
                certainty: Certainty::Exact,
                found_by: Vec::new(),
                unsure: Vec::new(),
                rows: Vec::new(),
                hidden_header: false,
            })
        }),
    }
}

fn list(entries: usize) -> Block {
    Block::List {
        ordered: false,
        items: (0..entries)
            .map(|i| vec![para(&format!("entry {i}"))])
            .collect(),
        anchor: None,
    }
}

fn section(kind: SectionKind, title: Option<&str>, blocks: Vec<Block>) -> Block {
    Block::Section {
        kind,
        title: title.map(str::to_string),
        anchor: title.map(|t| Anchor(t.to_string())),
        blocks,
    }
}

fn sheet(name: &str, blocks: Vec<Block>) -> Block {
    section(SectionKind::Sheet, Some(name), blocks)
}

fn formula(anchor: &str) -> Formula {
    Formula {
        anchor: Anchor(anchor.into()),
        expression: "=1+1".into(),
        dialect: "excel".into(),
        references: Vec::new(),
        cached: None,
        role: None,
        pattern: None,
        latex: None,
        note: None,
        kind: None,
        range: None,
        original: None,
    }
}

fn count(plan: &Plan, kind: ShapeKind) -> usize {
    plan.shapes.iter().filter(|s| s.kind == kind).count()
}

fn horizontal(s: &Shape) -> bool {
    s.kind == ShapeKind::Line && s.rect[1] == s.rect[3]
}

fn vertical(s: &Shape) -> bool {
    s.kind == ShapeKind::Line && s.rect[0] == s.rect[2]
}

/// Distinct positions, so a border drawn in pieces counts once.
fn distinct(mut v: Vec<f32>) -> usize {
    v.sort_by(f32::total_cmp);
    v.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    v.len()
}

/// The row borders of a plan: horizontal lines by their y.
fn row_borders(plan: &Plan) -> usize {
    distinct(
        plan.shapes
            .iter()
            .filter(|s| horizontal(s))
            .map(|s| s.rect[1])
            .collect(),
    )
}

/// The column borders of a plan: vertical lines by their x.
fn column_borders(plan: &Plan) -> usize {
    distinct(
        plan.shapes
            .iter()
            .filter(|s| vertical(s))
            .map(|s| s.rect[0])
            .collect(),
    )
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
}

/// The share of the inner area that walls and towers take at `w` by `h`, or `None` when
/// the plan does not fit.
fn share(plan: &Plan, w: u16, h: u16) -> Option<f32> {
    let f = fit(plan, w, h).ok()?;
    let m = 1 + MARGIN;
    let (gw, gh) = (f.grid.width(), f.grid.height());
    let mut taken = 0u32;
    for y in m..gh - m {
        for x in m..gw - m {
            if !f.grid.is_free(Pos { x, y }) {
                taken += 1;
            }
        }
    }
    Some(taken as f32 / (f32::from(gw - 2 * m) * f32::from(gh - 2 * m)))
}

/// A time bound in milliseconds: 50 in an optimized build, 500 with debug assertions, so a
/// loaded machine does not make a debug test run fail.
fn limit_ms() -> u128 {
    if cfg!(debug_assertions) { 500 } else { 50 }
}

fn costs() -> Document {
    let mut d = doc(vec![sheet("Costs", vec![table(5, 4, Some("A1:D5"))])]);
    d.facets.formulas = vec![formula("Costs!D5"), formula("Other!A1")];
    d
}

#[test]
fn sheet_gives_a_cell_grid() {
    let p = plans(&costs());
    assert_eq!(p.len(), 1);
    let p = &p[0];
    assert_eq!(p.label, "sheet Costs");
    assert_eq!(count(p, ShapeKind::Outline), 1);
    assert_eq!(row_borders(p), 4);
    assert_eq!(column_borders(p), 3);
    assert_eq!(count(p, ShapeKind::Tower), 1);
    let tower = p
        .shapes
        .iter()
        .find(|s| s.kind == ShapeKind::Tower)
        .unwrap();
    assert!(close(tower.rect, [0.75, 0.8, 1.0, 1.0]), "{:?}", tower.rect);
    // The outline comes first, so the cap on shapes never drops it.
    assert_eq!(p.shapes[0].kind, ShapeKind::Outline);
}

#[test]
fn csv_like_sheet_without_layout() {
    let p = plans(&doc(vec![sheet("data", vec![table(3, 4, None)])]));
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].label, "sheet data");
    assert_eq!(count(&p[0], ShapeKind::Outline), 1);
    assert_eq!(row_borders(&p[0]), 2);
    assert_eq!(column_borders(&p[0]), 3);
}

#[test]
fn sheets_become_rounds() {
    let d = doc(vec![
        sheet("Costs", vec![table(5, 4, Some("A1:D5"))]),
        sheet("Notes", vec![table(2, 2, Some("B2:C3"))]),
    ]);
    let labels: Vec<_> = plans(&d).into_iter().map(|p| p.label).collect();
    assert_eq!(labels, ["sheet Costs", "sheet Notes"]);
}

#[test]
fn huge_sheet_range_is_clipped() {
    let started = Instant::now();
    let d = doc(vec![sheet("Big", vec![table(3, 3, Some("A1:XFD1048576"))])]);
    let p = plans(&d);
    assert!(
        started.elapsed().as_millis() < limit_ms(),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(p.len(), 1);
    assert_eq!(count(&p[0], ShapeKind::Outline), 1);
    assert!(row_borders(&p[0]) <= 12, "{}", row_borders(&p[0]));
    assert!(column_borders(&p[0]) <= 8, "{}", column_borders(&p[0]));
    assert!(row_borders(&p[0]) >= 4 && column_borders(&p[0]) >= 3);
    assert!(fit(&p[0], 80, 40).is_ok());
}

#[test]
fn many_formulas_are_spread_and_capped() {
    let mut d = doc(vec![sheet("Model", vec![table(200, 60, Some("A1:BH200"))])]);
    d.facets.formulas = (1..=200)
        .flat_map(|r| (0..60).map(move |c| (r, c)))
        .map(|(r, c)| formula(&format!("Model!{}{r}", rezzer_model::column_letter(c))))
        .collect();
    let p = plans(&d);
    let towers = count(&p[0], ShapeKind::Tower);
    assert!((1..=64).contains(&towers), "{towers}");
    assert!(fit(&p[0], 80, 40).is_ok());
}

#[test]
fn crowded_sheet_falls_back_to_the_generic_plan() {
    // Many small regions side by side: each outline is too short for a door and seals solid.
    let regions = (0..20)
        .flat_map(|r| (0..10).map(move |c| (r, c)))
        .map(|(r, c)| {
            let col = rezzer_model::column_letter(c * 2);
            let to = rezzer_model::column_letter(c * 2 + 1);
            let range = format!("{col}{}:{to}{}", r * 2 + 1, r * 2 + 2);
            table(2, 2, Some(&range))
        })
        .collect();
    let p = plans(&doc(vec![sheet("Dense", regions)]));
    assert_eq!(p[0].label, "sheet Dense, part 1 of 13");
    assert!(fit(&p[0], 80, 40).is_ok());
}

#[test]
fn quoted_sheet_names_match_their_formulas() {
    let mut d = doc(vec![Block::Section {
        kind: SectionKind::Sheet,
        title: Some("My Costs".into()),
        anchor: Some(Anchor("'My Costs'".into())),
        blocks: vec![table(2, 2, Some("A1:B2"))],
    }]);
    d.facets.formulas = vec![formula("'My Costs'!B2")];
    let p = plans(&d);
    assert_eq!(p[0].label, "sheet My Costs");
    assert_eq!(count(&p[0], ShapeKind::Tower), 1);
}

#[test]
fn text_gives_towers_for_headings() {
    let d = doc(vec![
        heading(1, "Costs of the year"),
        para("A short paragraph."),
        heading(2, "Costs of the year"),
        table(3, 3, None),
        list(3),
        para(&"long words ".repeat(40)),
    ]);
    let p = plans(&d);
    assert_eq!(p.len(), 1);
    let p = &p[0];
    assert_eq!(p.label, "Costs of the year");
    let towers: Vec<_> = p
        .shapes
        .iter()
        .filter(|s| s.kind == ShapeKind::Tower)
        .collect();
    assert_eq!(towers.len(), 2);
    // Item 0 is left-aligned, item 2 too; the deeper heading is 0.05 narrower.
    assert!((towers[0].rect[0] - 0.05).abs() < 1e-6);
    let w0 = towers[0].rect[2] - towers[0].rect[0];
    let w1 = towers[1].rect[2] - towers[1].rect[0];
    assert!((w0 - w1 - 0.05).abs() < 1e-4, "{w0} {w1}");
    assert_eq!(count(p, ShapeKind::Outline), 1);
    // The table in a region with other items: at most 4 row and 3 column borders.
    assert_eq!(row_borders_in(p, 3), 2);
    assert_eq!(column_borders(p), 2);
    // Two paragraphs and the list give lines of their own, one per item.
    let item_lines: Vec<_> = p
        .shapes
        .iter()
        .filter(|s| horizontal(s) && in_slot(s, 6, &[1, 4, 5]))
        .collect();
    assert_eq!(item_lines.len(), 3);
    let width = |s: &Shape| s.rect[2] - s.rect[0];
    let short = item_lines.iter().find(|s| in_slot(s, 6, &[1])).unwrap();
    assert!((width(short) - 0.08).abs() < 1e-4);
    let long = item_lines.iter().find(|s| in_slot(s, 6, &[5])).unwrap();
    assert!((width(long) - 0.4).abs() < 1e-4);
    let entries = item_lines.iter().find(|s| in_slot(s, 6, &[4])).unwrap();
    assert!((width(entries) - 0.19).abs() < 1e-4);
    // Odd items end at 0.95.
    assert!((short.rect[2] - 0.95).abs() < 1e-6);
}

/// True when `s` sits in one of the item slots `slots` of `n`.
fn in_slot(s: &Shape, n: usize, slots: &[usize]) -> bool {
    let y = (s.rect[1] + s.rect[3]) / 2.0;
    slots
        .iter()
        .any(|&i| (y - (i as f32 + 0.5) / n as f32).abs() < 1e-4)
}

/// The row borders inside item slot `slot` of 6: horizontal lines that are not on the slot
/// centre and not the outline.
fn row_borders_in(p: &Plan, slot: usize) -> usize {
    let (lo, hi) = (slot as f32 / 6.0, (slot + 1) as f32 / 6.0);
    distinct(
        p.shapes
            .iter()
            .filter(|s| {
                horizontal(s) && s.rect[1] > lo && s.rect[1] < hi && !in_slot(s, 6, &[slot])
            })
            .map(|s| s.rect[1])
            .collect(),
    )
}

#[test]
fn long_text_is_chunked() {
    let body = (0..40)
        .map(|i| heading(1, &format!("Heading {i}")))
        .collect();
    let p = plans(&doc(body));
    let labels: Vec<_> = p.iter().map(|p| p.label.as_str()).collect();
    assert_eq!(labels, ["Heading 0", "Heading 16", "Heading 32"]);
    assert_eq!(count(&p[0], ShapeKind::Tower), 16);
    assert_eq!(count(&p[2], ShapeKind::Tower), 8);
}

#[test]
fn plans_are_capped() {
    let body = (0..2000).map(|i| para(&format!("p {i}"))).collect();
    assert_eq!(plans(&doc(body)).len(), MAX_PLANS);
}

#[test]
fn long_labels_are_shortened() {
    let p = plans(&doc(vec![heading(1, &"x".repeat(80))]));
    assert_eq!(p[0].label, "x".repeat(30));
}

#[test]
fn generic_plan_for_any_sections() {
    let d = doc(vec![
        section(
            SectionKind::Slide,
            Some("Intro"),
            vec![heading(1, "Welcome"), table(4, 3, None), list(2)],
        ),
        section(
            SectionKind::Chapter,
            None,
            vec![
                Block::Quote {
                    blocks: vec![para("said"), para("twice")],
                    anchor: None,
                },
                Block::Media {
                    asset: "img1".into(),
                    alt: None,
                    anchor: None,
                },
                section(
                    SectionKind::Other,
                    Some("inner"),
                    vec![heading(2, "Inside")],
                ),
            ],
        ),
    ]);
    let p = plans(&d);
    let labels: Vec<_> = p.iter().map(|p| p.label.as_str()).collect();
    assert_eq!(labels, ["slide Intro", "chapter 1"]);
    assert_eq!(count(&p[0], ShapeKind::Tower), 1);
    assert_eq!(count(&p[0], ShapeKind::Outline), 1);
    assert_eq!(column_borders(&p[0]), 2);
    // Quote run, picture and the nested section's heading, opened inline.
    assert_eq!(count(&p[1], ShapeKind::Block), 1);
    assert_eq!(count(&p[1], ShapeKind::Tower), 1);
    assert_eq!(count(&p[1], ShapeKind::Line), 1);
}

#[test]
fn a_lone_table_fills_its_region() {
    let p = plans(&doc(vec![table(5, 4, None)]));
    assert_eq!(count(&p[0], ShapeKind::Outline), 1);
    let outline = p[0].shapes[0];
    assert!(close(outline.rect, [0.0, 0.0, 1.0, 1.0]));
    assert_eq!(row_borders(&p[0]), 4);
    assert_eq!(column_borders(&p[0]), 3);
}

#[test]
fn loose_blocks_beside_sections_are_a_region() {
    let d = doc(vec![
        para("before"),
        section(SectionKind::Page, Some("One"), vec![para("on the page")]),
        section(SectionKind::Other, None, vec![para("other")]),
    ]);
    let labels: Vec<_> = plans(&d).into_iter().map(|p| p.label).collect();
    assert_eq!(labels, ["part 1 of 1", "page One", "section 1"]);
}

#[test]
fn sheet_without_a_table_uses_the_generic_plan() {
    let p = plans(&doc(vec![sheet("Empty", vec![para("no cells here")])]));
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].label, "sheet Empty");
    assert_eq!(count(&p[0], ShapeKind::Line), 1);
}

#[test]
fn prose_only_gives_a_plan() {
    let body = (0..5).map(|i| para(&format!("Sentence {i}."))).collect();
    let p = plans(&doc(body));
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].label, "part 1 of 1");
    assert_eq!(count(&p[0], ShapeKind::Line), 5);
    let r = rounds(
        "prose.txt",
        Some(&doc((0..3).map(|_| para("x")).collect())),
        None,
    );
    assert_ne!(r.plans[0].label, "the Grid");
    assert_eq!(r.note, None);
}

#[test]
fn blank_paragraphs_give_nothing() {
    assert!(plans(&doc(vec![para("   "), para("")])).is_empty());
}

#[test]
fn every_kind_of_item_at_its_largest_stays_under_the_cap() {
    let long = "w".repeat(1000);
    let kinds: Vec<(&str, Block)> = vec![
        ("heading", heading(1, &long)),
        ("paragraph", para(&long)),
        ("list", list(50)),
        (
            "quote",
            Block::Quote {
                blocks: vec![para("q"); 50],
                anchor: None,
            },
        ),
        (
            "math",
            Block::Math {
                latex: long.clone(),
                source_kind: "latex".into(),
                anchor: None,
            },
        ),
        (
            "code",
            Block::Code {
                lang: None,
                text: long.clone(),
                anchor: None,
            },
        ),
        (
            "media",
            Block::Media {
                asset: "a".into(),
                alt: None,
                anchor: None,
            },
        ),
        ("rule", Block::Rule { anchor: None }),
        ("table", table(50, 20, None)),
    ];
    for (name, block) in kinds {
        let p = plans(&doc(vec![block; 16]));
        assert_eq!(p.len(), 1, "{name}");
        let s = share(&p[0], 80, 40);
        assert!(s.is_some_and(|s| s < 0.40), "{name}: {s:?}");
    }
}

/// An invented chapter of prose with the usual mix of items.
fn mixed() -> Document {
    let mut body = Vec::new();
    for i in 0..4 {
        body.push(heading(
            1 + (i % 2) as u8,
            &format!("Chapter {i} of the invented story"),
        ));
        body.push(para(&"word ".repeat(120)));
        body.push(para(&"word ".repeat(30)));
        body.push(list(4));
        body.push(Block::Task {
            state: TaskState::Open,
            inlines: text("a task"),
            anchor: None,
        });
        body.push(Block::Task {
            state: TaskState::Done,
            inlines: text("another task"),
            anchor: None,
        });
        body.push(table(6, 4, None));
        body.push(Block::Code {
            lang: None,
            text: "x".into(),
            anchor: None,
        });
        body.push(para(&"word ".repeat(400)));
    }
    doc(body)
}

#[test]
fn a_task_run_is_one_item() {
    let d = doc(vec![
        Block::Task {
            state: TaskState::Open,
            inlines: text("one"),
            anchor: None,
        },
        Block::Task {
            state: TaskState::Done,
            inlines: text("two"),
            anchor: None,
        },
    ]);
    let p = plans(&d);
    assert_eq!(count(&p[0], ShapeKind::Line), 1);
    assert!((p[0].shapes[0].rect[2] - p[0].shapes[0].rect[0] - 0.16).abs() < 1e-4);
}

#[test]
fn a_usual_mix_stays_well_under_the_cap() {
    let p = plans(&mixed());
    assert_eq!(p.len(), 2);
    for plan in &p {
        let s = share(plan, 80, 40).expect("fits at 80x40");
        assert!(s < 0.30, "{}: {s}", plan.label);
    }
}

#[test]
fn every_plan_fits() {
    let docs = [
        costs(),
        doc(vec![sheet("data", vec![table(3, 4, None)])]),
        doc(vec![sheet("Big", vec![table(3, 3, Some("A1:XFD1048576"))])]),
        doc(vec![sheet("Long", vec![table(5000, 12, None)])]),
        doc((0..40)
            .map(|i| heading(1, &format!("Heading {i}")))
            .collect()),
        doc((0..100).map(|_| para(&"word ".repeat(80))).collect()),
        mixed(),
        doc(vec![table(5, 4, None)]),
    ];
    for d in &docs {
        for p in plans(d) {
            assert!(fit(&p, 80, 40).is_ok(), "{}", p.label);
        }
    }
}

#[test]
fn empty_document_gives_the_grid_with_a_note() {
    let d = doc(Vec::new());
    let r = rounds("empty.txt", Some(&d), None);
    assert_eq!(r.name, "empty.txt");
    assert_eq!(r.plans.len(), 1);
    assert_eq!(r.plans[0].label, "the Grid");
    assert_eq!(
        r.note.as_deref(),
        Some("empty.txt gives no grid: nothing to build a grid from. Using the Grid.")
    );
}

#[test]
fn unreadable_gives_the_grid_with_the_reason() {
    let r = rounds("x.bin", None, Some("no reader for this format"));
    assert_eq!(r.plans.len(), 1);
    assert_eq!(r.plans[0].label, "the Grid");
    assert_eq!(
        r.note.as_deref(),
        Some("x.bin gives no grid: no reader for this format. Using the Grid.")
    );
    let r = rounds("y.bin", None, None);
    assert_eq!(
        r.note.as_deref(),
        Some("y.bin gives no grid: nothing to build a grid from. Using the Grid.")
    );
}

#[test]
fn no_file_gives_the_grid_without_a_note() {
    let r = rounds("", None, None);
    assert_eq!(r.name, "");
    assert_eq!(r.plans.len(), 1);
    assert_eq!(r.plans[0].label, "the Grid");
    assert_eq!(r.note, None);
}

#[test]
fn a_sheet_document_gives_rounds() {
    let r = rounds("costs.xlsx", Some(&costs()), None);
    assert_eq!(r.plans.len(), 1);
    assert_eq!(r.plans[0].label, "sheet Costs");
    assert_eq!(r.note, None);
}

#[test]
fn sections_count_by_kind() {
    let d = doc(vec![
        para("a title block"),
        section(SectionKind::Slide, None, vec![para("one")]),
        section(SectionKind::Chapter, None, vec![para("two")]),
        section(SectionKind::Slide, None, vec![para("three")]),
    ]);
    let labels: Vec<_> = plans(&d).into_iter().map(|p| p.label).collect();
    assert_eq!(labels, ["part 1 of 1", "slide 1", "chapter 1", "slide 2"]);
}

#[test]
fn formulas_find_their_sheet_among_many() {
    let sheets: Vec<_> = (0..50)
        .map(|i| sheet(&format!("S{i}"), vec![table(4, 4, Some("A1:D4"))]))
        .collect();
    let mut d = doc(sheets);
    d.facets.formulas = (0..50)
        .flat_map(|i| (1..=i % 4 + 1).map(move |r| formula(&format!("S{i}!A{r}"))))
        .collect();
    let p = plans(&d);
    assert_eq!(p.len(), 50);
    for (i, plan) in p.iter().enumerate() {
        assert_eq!(count(plan, ShapeKind::Tower), i % 4 + 1, "{}", plan.label);
    }
}

/// Takes a nested section apart one section at a time, so dropping it needs no deep recursion.
fn drop_flat(mut block: Block) {
    while let Block::Section { blocks, .. } = &mut block {
        let Some(inner) = blocks.pop() else {
            break;
        };
        block = inner;
    }
}

#[test]
fn deeply_nested_sections_do_not_overflow() {
    let mut block = para("deep inside");
    for _ in 0..10_000 {
        block = section(SectionKind::Other, None, vec![block]);
    }
    let mut d = doc(vec![block]);
    let started = Instant::now();
    let p = plans(&d);
    assert!(
        started.elapsed().as_millis() < limit_ms(),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].label, "section 1");
    assert_eq!(count(&p[0], ShapeKind::Line), 1);
    drop_flat(d.body.pop().unwrap());
}

#[test]
fn a_long_body_is_fast() {
    let body: Vec<_> = (0..100_000)
        .map(|i| match i % 3 {
            0 => heading(1, &format!("Heading {i}")),
            1 => para(&"word ".repeat(20)),
            _ => list(3),
        })
        .collect();
    let d = doc(body);
    let started = Instant::now();
    let p = plans(&d);
    assert!(
        started.elapsed().as_millis() < limit_ms(),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(p.len(), MAX_PLANS);
}

#[test]
fn a_large_table_among_other_items_is_capped() {
    let p = plans(&doc(vec![heading(1, "Before"), table(500, 100, None)]));
    let p = &p[0];
    assert_eq!(column_borders(p), 3);
    let inside: Vec<f32> = p
        .shapes
        .iter()
        .filter(|s| horizontal(s) && !in_slot(s, 2, &[0, 1]))
        .map(|s| s.rect[1])
        .collect();
    assert_eq!(distinct(inside), 4);
}

#[test]
fn a_title_with_a_quote_matches_its_quoted_formulas() {
    // No anchor: the title is the only name, and the formula doubles the quote inside.
    let mut d = doc(vec![Block::Section {
        kind: SectionKind::Sheet,
        title: Some("Q1 '24".into()),
        anchor: None,
        blocks: vec![table(5, 4, Some("A1:D5"))],
    }]);
    d.facets.formulas = vec![formula("'Q1 ''24'!A1"), formula("'Q1 ''24'!D5")];
    let p = plans(&d);
    assert_eq!(p[0].label, "sheet Q1 '24");
    assert_eq!(count(&p[0], ShapeKind::Tower), 2);
}
