// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::time::Instant;

use rezzer_derez::grid::Pos;
use rezzer_derez::plan::{
    MARGIN, MAX_PLANS, MAX_SHAPES, Plan, Shape, ShapeKind, fit, plans, rounds,
};
use rezzer_model::{
    Anchor, Block, BoxKind, Cell, Certainty, Confidence, Detection, Document, FormatId, Formula,
    Geometry, Inline, LayoutKind, PageGeometry, PlacedBox, SectionKind, SourceInfo, TableLayout,
    TaskState,
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
    assert_eq!(
        r.short_note.as_deref(),
        Some("No grid from this file: nothing to build a grid from.")
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
    assert_eq!(
        r.short_note.as_deref(),
        Some("No grid from this file: no reader for this format.")
    );
    // A reason that ends with a full stop gets no second one.
    let r = rounds("z.bin", None, Some("the file is broken."));
    assert_eq!(
        r.note.as_deref(),
        Some("z.bin gives no grid: the file is broken. Using the Grid.")
    );
    assert_eq!(
        r.short_note.as_deref(),
        Some("No grid from this file: the file is broken.")
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
    assert_eq!(r.short_note, None);
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

fn placed(kind: BoxKind, bounds: [f32; 4]) -> PlacedBox {
    PlacedBox {
        kind,
        bounds,
        anchor: None,
    }
}

/// A page of 600 by 800 points with `boxes`, built past the checks of `Geometry::push`.
fn page(page: u32, boxes: Vec<PlacedBox>) -> PageGeometry {
    PageGeometry {
        page,
        size: [600.0, 800.0],
        boxes,
    }
}

fn with_pages(body: Vec<Block>, pages: Vec<PageGeometry>) -> Document {
    let mut d = doc(body);
    let mut g = Geometry::default();
    g.pages = pages;
    d.geometry = Some(Box::new(g));
    d
}

/// An invented PDF-like page: a rule, a table of four cells, a picture, a heading and text.
fn pdf_like_page() -> Vec<PlacedBox> {
    vec![
        placed(BoxKind::Heading(1), [50.0, 740.0, 400.0, 760.0]),
        placed(BoxKind::Rule, [50.0, 700.0, 550.0, 700.0]),
        placed(BoxKind::Text, [50.0, 600.0, 550.0, 650.0]),
        placed(BoxKind::Table, [100.0, 300.0, 500.0, 500.0]),
        placed(BoxKind::Cell, [100.0, 400.0, 300.0, 500.0]),
        placed(BoxKind::Cell, [300.0, 400.0, 500.0, 500.0]),
        placed(BoxKind::Cell, [100.0, 300.0, 300.0, 400.0]),
        placed(BoxKind::Cell, [300.0, 300.0, 500.0, 400.0]),
        placed(BoxKind::Picture, [400.0, 100.0, 500.0, 200.0]),
    ]
}

#[test]
fn pdf_like_page_gives_walls_and_towers() {
    let mut d = doc(Vec::new());
    let mut g = Geometry::default();
    assert!(g.add_page(1, [600.0, 800.0]));
    for b in pdf_like_page() {
        assert!(g.push(1, b));
    }
    d.geometry = Some(Box::new(g));
    let ps = plans(&d);
    assert_eq!(ps.len(), 1);
    let p = &ps[0];
    assert_eq!(p.label, "page 1 of 1");
    // The rule, and the cells as the inner borders of the table: one column line and one
    // row border in two pieces. The table is the one outline.
    assert_eq!(count(p, ShapeKind::Line), 4);
    assert_eq!(count(p, ShapeKind::Outline), 1);
    assert_eq!(count(p, ShapeKind::Block), 1);
    assert_eq!(count(p, ShapeKind::Tower), 1);
    assert_eq!(p.shapes.len(), 7);
    assert_eq!(column_borders(p), 1);
    assert_eq!(row_borders(p), 2);
    let tower = p
        .shapes
        .iter()
        .find(|s| s.kind == ShapeKind::Tower)
        .unwrap();
    // y is flipped: the heading at the top of the page is near y 0.
    assert!(tower.rect[3] < 0.1, "{:?}", tower.rect);
    let rule = p.shapes.iter().find(|s| s.kind == ShapeKind::Line).unwrap();
    assert!(close(
        p.shapes
            .iter()
            .find(|s| s.kind == ShapeKind::Outline)
            .unwrap()
            .rect,
        [100.0 / 600.0, 0.375, 500.0 / 600.0, 0.625]
    ));
    assert!(close(
        rule.rect,
        [50.0 / 600.0, 0.125, 550.0 / 600.0, 0.125]
    ));
    let picture = p
        .shapes
        .iter()
        .find(|s| s.kind == ShapeKind::Block)
        .unwrap();
    assert!(close(
        picture.rect,
        [400.0 / 600.0, 0.75, 500.0 / 600.0, 0.875]
    ));
    assert!(fit(p, 80, 40).is_ok());
}

#[test]
fn pages_become_rounds() {
    let rule = || vec![placed(BoxKind::Rule, [50.0, 400.0, 550.0, 400.0])];
    let pages = vec![page(1, rule()), page(2, Vec::new()), page(3, rule())];
    let d = with_pages(Vec::new(), pages.clone());
    let labels: Vec<String> = plans(&d).into_iter().map(|p| p.label).collect();
    assert_eq!(labels, ["page 1 of 3", "page 3 of 3"]);
    // The page count of the file wins over the largest page with geometry.
    let mut d = with_pages(Vec::new(), pages);
    d.properties.facts.pages = Some(5);
    let labels: Vec<String> = plans(&d).into_iter().map(|p| p.label).collect();
    assert_eq!(labels, ["page 1 of 5", "page 3 of 5"]);
}

#[test]
fn page_plans_are_capped() {
    let pages = (1..=80)
        .map(|n| page(n, vec![placed(BoxKind::Rule, [50.0, 400.0, 550.0, 400.0])]))
        .collect();
    let ps = plans(&with_pages(Vec::new(), pages));
    assert_eq!(ps.len(), MAX_PLANS);
    assert_eq!(ps[0].label, "page 1 of 80");
}

#[test]
fn degenerate_geometry_is_ignored() {
    let rule = placed(BoxKind::Rule, [50.0, 400.0, 550.0, 400.0]);
    let pages = vec![
        PageGeometry {
            page: 1,
            size: [f32::NAN, 800.0],
            boxes: vec![rule.clone()],
        },
        PageGeometry {
            page: 2,
            size: [0.0, 800.0],
            boxes: vec![rule.clone()],
        },
        PageGeometry {
            page: 3,
            size: [600.0, f32::INFINITY],
            boxes: vec![rule.clone()],
        },
        page(
            4,
            vec![
                placed(BoxKind::Drawn, [100.0, 500.0, 200.0, 400.0]),
                placed(BoxKind::Drawn, [200.0, 100.0, 100.0, 200.0]),
                placed(BoxKind::Drawn, [1.0e6, 1.0e6, 2.0e6, 2.0e6]),
                placed(BoxKind::Rule, [-900.0, -50.0, -100.0, -50.0]),
                placed(BoxKind::Drawn, [0.0, 0.0, f32::INFINITY, 100.0]),
                placed(BoxKind::Picture, [f32::NAN, 0.0, 100.0, 100.0]),
                placed(BoxKind::Heading(0), [-50.0, 700.0, 300.0, 700.0]),
                rule.clone(),
            ],
        ),
    ];
    let ps = plans(&with_pages(Vec::new(), pages));
    assert_eq!(ps.len(), 1);
    assert_eq!(ps[0].label, "page 4 of 4");
    // Only the rule and the heading, clipped to the page and raised, are valid.
    assert_eq!(ps[0].shapes.len(), 2, "{:?}", ps[0].shapes);
    assert_eq!(count(&ps[0], ShapeKind::Line), 1);
    let tower = ps[0]
        .shapes
        .iter()
        .find(|s| s.kind == ShapeKind::Tower)
        .unwrap();
    let [x0, y0, x1, y1] = tower.rect;
    assert!(x0 >= 0.0 && x1 <= 1.0 && x0 < x1, "{:?}", tower.rect);
    assert!(y1 - y0 >= 0.02 - 1e-6, "{:?}", tower.rect);
    assert!(fit(&ps[0], 80, 40).is_ok());

    // 100,000 rules spread over one page: no panic; the rules that are drawn crowd the
    // page, so it gives the Grid with a note.
    let many = (0..100_000u32)
        .map(|i| {
            let y = 800.0 * ((i * 7919) % 100_000) as f32 / 100_000.0;
            placed(BoxKind::Rule, [0.0, y, 600.0, y])
        })
        .collect();
    let d = with_pages(Vec::new(), vec![page(1, many)]);
    assert_eq!(plans(&d), Vec::new());
    let r = rounds("ruled.pdf", Some(&d), None);
    assert_eq!(r.plans.len(), 1);
    assert_eq!(r.plans[0].label, "the Grid");
    assert_eq!(
        r.note.as_deref(),
        Some("ruled.pdf gives no grid: nothing to build a grid from. Using the Grid.")
    );

    // More boxes than a plan draws: the shapes stop at the cap.
    let same = vec![placed(BoxKind::Drawn, [100.0, 100.0, 300.0, 300.0]); MAX_SHAPES + 500];
    let ps = plans(&with_pages(Vec::new(), vec![page(1, same)]));
    assert_eq!(ps[0].shapes.len(), MAX_SHAPES);

    // A tiny page with bounds as large as an f32 allows: quotients overflow to infinity.
    let huge = PageGeometry {
        page: 1,
        size: [1.0e-30, 1.0e-30],
        boxes: vec![placed(
            BoxKind::Picture,
            [-f32::MAX, -f32::MAX, f32::MAX, f32::MAX],
        )],
    };
    let ps = plans(&with_pages(Vec::new(), vec![huge]));
    assert_eq!(ps.len(), 1);
    assert_eq!(ps[0].shapes[0].kind, ShapeKind::Outline);
    assert!(close(ps[0].shapes[0].rect, [0.0, 0.0, 1.0, 1.0]));
}

#[test]
fn full_page_picture_is_an_outline() {
    let ps = plans(&with_pages(
        Vec::new(),
        vec![page(
            1,
            vec![placed(BoxKind::Picture, [0.0, 0.0, 600.0, 800.0])],
        )],
    ));
    assert_eq!(ps.len(), 1);
    assert_eq!(ps[0].shapes.len(), 1);
    assert_eq!(ps[0].shapes[0].kind, ShapeKind::Outline);
    assert!(close(ps[0].shapes[0].rect, [0.0, 0.0, 1.0, 1.0]));
    assert!(fit(&ps[0], 80, 40).is_ok());
    // The share counts the part on the page: half of this picture is off the page.
    let ps = plans(&with_pages(
        Vec::new(),
        vec![page(
            1,
            vec![placed(BoxKind::Picture, [-600.0, 0.0, 300.0, 800.0])],
        )],
    ));
    assert_eq!(ps[0].shapes[0].kind, ShapeKind::Outline);
    assert!(close(ps[0].shapes[0].rect, [0.0, 0.0, 0.5, 1.0]));
    // A fifth of the page stays a block.
    let ps = plans(&with_pages(
        Vec::new(),
        vec![page(
            1,
            vec![placed(BoxKind::Picture, [0.0, 0.0, 120.0, 800.0])],
        )],
    ));
    assert_eq!(ps[0].shapes[0].kind, ShapeKind::Block);
}

#[test]
fn headings_are_cut_by_depth() {
    let wide = |depth| placed(BoxKind::Heading(depth), [0.0, 400.0, 600.0, 400.0]);
    let ps = plans(&with_pages(
        Vec::new(),
        vec![page(1, vec![wide(1), wide(3), wide(9)])],
    ));
    let widths: Vec<f32> = ps[0].shapes.iter().map(|s| s.rect[2] - s.rect[0]).collect();
    assert_eq!(widths.len(), 3);
    for (w, want) in widths.iter().zip([0.6, 0.4, 0.2]) {
        assert!((w - want).abs() < 1e-4, "{widths:?}");
    }
    for s in &ps[0].shapes {
        assert_eq!(s.kind, ShapeKind::Tower);
        assert!(s.rect[3] - s.rect[1] >= 0.02 - 1e-6, "{:?}", s.rect);
    }
    // A heading on the bottom edge is raised inside the page.
    let ps = plans(&with_pages(
        Vec::new(),
        vec![page(
            1,
            vec![placed(BoxKind::Heading(2), [0.0, 0.0, 100.0, 0.0])],
        )],
    ));
    let [_, y0, _, y1] = ps[0].shapes[0].rect;
    assert!(y1 <= 1.0 && (y1 - y0 - 0.02).abs() < 1e-4, "{y0} {y1}");
}

#[test]
fn geometry_wins_over_text() {
    let body: Vec<Block> = (0..20)
        .map(|i| heading(1, &format!("Heading {i}")))
        .collect();
    let d = with_pages(
        body.clone(),
        vec![page(
            1,
            vec![placed(BoxKind::Rule, [50.0, 400.0, 550.0, 400.0])],
        )],
    );
    let labels: Vec<String> = plans(&d).into_iter().map(|p| p.label).collect();
    assert_eq!(labels, ["page 1 of 1"]);
    // Geometry with nothing but text gives no shape, so the text gives the plans.
    let d = with_pages(
        body.clone(),
        vec![page(
            1,
            vec![placed(BoxKind::Text, [50.0, 400.0, 550.0, 450.0])],
        )],
    );
    assert_eq!(plans(&d), plans(&doc(body)));
}

/// Pictures side by side that fill the page: crowded at 80 by 40.
fn tiles() -> Vec<PlacedBox> {
    (0..30)
        .flat_map(|r| {
            (0..30).map(move |c| {
                let (x, y) = (20.0 * c as f32, 800.0 / 30.0 * r as f32);
                placed(BoxKind::Picture, [x, y, x + 19.0, y + 26.0])
            })
        })
        .collect()
}

#[test]
fn crowded_pages_are_left_out() {
    let tiles = tiles();
    let rule = vec![placed(BoxKind::Rule, [50.0, 400.0, 550.0, 400.0])];
    let d = with_pages(Vec::new(), vec![page(1, tiles), page(2, rule)]);
    let ps = plans(&d);
    let labels: Vec<&str> = ps.iter().map(|p| p.label.as_str()).collect();
    assert_eq!(labels, ["page 2 of 2"]);
}

#[test]
fn all_pages_crowded_gives_the_text_plans() {
    let body: Vec<Block> = (0..20)
        .map(|i| heading(1, &format!("Heading {i}")))
        .collect();
    let d = with_pages(body.clone(), vec![page(1, tiles()), page(2, tiles())]);
    let ps = plans(&d);
    assert!(!ps.is_empty());
    assert_eq!(ps, plans(&doc(body)));
}

#[test]
fn pages_tried_are_capped() {
    // 200 crowded pages use up the pages tried, so the good page after them is not reached.
    let rule = vec![placed(BoxKind::Rule, [50.0, 400.0, 550.0, 400.0])];
    let mut pages: Vec<PageGeometry> = (1..=4 * MAX_PLANS as u32)
        .map(|n| page(n, tiles()))
        .collect();
    pages.push(page(4 * MAX_PLANS as u32 + 1, rule.clone()));
    assert_eq!(plans(&with_pages(Vec::new(), pages)), Vec::new());
    // One page fewer, and it is.
    let mut pages: Vec<PageGeometry> = (1..4 * MAX_PLANS as u32)
        .map(|n| page(n, tiles()))
        .collect();
    pages.push(page(4 * MAX_PLANS as u32, rule));
    let labels: Vec<String> = plans(&with_pages(Vec::new(), pages))
        .into_iter()
        .map(|p| p.label)
        .collect();
    assert_eq!(labels, ["page 200 of 200"]);
}

#[test]
fn many_crowded_pages_are_fast() {
    let pages: Vec<PageGeometry> = (1..=5000).map(|n| page(n, tiles())).collect();
    let d = with_pages(Vec::new(), pages);
    let start = Instant::now();
    let ps = plans(&d);
    let took = start.elapsed().as_millis();
    assert!(ps.is_empty());
    // Only the first 200 pages with shapes are tried: about 16 ms with debug assertions and
    // 8 ms optimized here, against 500 and 50.
    assert!(took < limit_ms(), "{took} ms");
}

/// The free cells of `g` that `from` reaches.
fn reached(g: &rezzer_derez::grid::Grid, from: Pos) -> Vec<bool> {
    use rezzer_derez::grid::Dir;
    let w = usize::from(g.width());
    let mut seen = vec![false; w * usize::from(g.height())];
    let mut todo = vec![from];
    seen[usize::from(from.y) * w + usize::from(from.x)] = true;
    while let Some(p) = todo.pop() {
        for d in Dir::ALL {
            if let Some(q) = g.step(p, d).filter(|&q| g.is_free(q)) {
                let i = usize::from(q.y) * w + usize::from(q.x);
                if !seen[i] {
                    seen[i] = true;
                    todo.push(q);
                }
            }
        }
    }
    seen
}

/// A table of `rows` by `cols` cells over the lower half of a 600 by 800 page.
fn half_page_table(rows: u32, cols: u32) -> Vec<PlacedBox> {
    let (x0, y0, x1, y1) = (60.0, 40.0, 540.0, 400.0);
    let mut boxes = vec![placed(BoxKind::Table, [x0, y0, x1, y1])];
    let (cw, ch) = ((x1 - x0) / cols as f32, (y1 - y0) / rows as f32);
    for r in 0..rows {
        for c in 0..cols {
            let (cx, cy) = (x0 + cw * c as f32, y0 + ch * r as f32);
            boxes.push(placed(BoxKind::Cell, [cx, cy, cx + cw, cy + ch]));
        }
    }
    boxes
}

#[test]
fn table_cells_stay_open() {
    for (rows, cols) in [(2, 2), (6, 4)] {
        let ps = plans(&with_pages(
            Vec::new(),
            vec![page(1, half_page_table(rows, cols))],
        ));
        assert_eq!(ps.len(), 1, "{rows}x{cols}");
        let p = &ps[0];
        assert_eq!(count(p, ShapeKind::Outline), 1, "{rows}x{cols}");
        assert_eq!(column_borders(p), cols as usize - 1, "{rows}x{cols}");
        assert_eq!(row_borders(p), rows as usize - 1, "{rows}x{cols}");
        let f = fit(p, 80, 40).unwrap();
        let seen = reached(&f.grid, f.starts[0].0);
        // The cells strictly inside the outline of the table rect (x 0.1..0.9, y 0.5..0.95):
        // more than two thirds stay free and reached (2x2: 702 of 741, 6x4: 578 of 741).
        let m = 1 + MARGIN;
        let (iw, ih) = (f.grid.width() - 2 * m, f.grid.height() - 2 * m);
        let cell = |u: f32, n: u16| m + (u * f32::from(n)).floor() as u16;
        let (gx0, gx1) = (cell(0.1, iw) + 1, cell(0.9, iw) - 1);
        let (gy0, gy1) = (cell(0.5, ih) + 1, cell(0.95, ih) - 1);
        let (mut inside, mut open) = (0, 0);
        for y in gy0..gy1 {
            for x in gx0..gx1 {
                inside += 1;
                let at = usize::from(y) * usize::from(f.grid.width()) + usize::from(x);
                if f.grid.is_free(Pos { x, y }) && seen[at] {
                    open += 1;
                }
            }
        }
        assert!(
            open * 3 > inside * 2,
            "{rows}x{cols}: {open} of {inside} reached"
        );
    }
}

#[test]
fn table_borders_are_capped_and_loose_cells_are_outlines() {
    // 40 by 30 cells: at most 8 row and 6 column bands are shown.
    let ps = plans(&with_pages(
        Vec::new(),
        vec![page(1, half_page_table(40, 30))],
    ));
    assert!(row_borders(&ps[0]) <= 7, "{}", row_borders(&ps[0]));
    assert!(column_borders(&ps[0]) <= 5, "{}", column_borders(&ps[0]));
    assert!(fit(&ps[0], 80, 40).is_ok());
    // A cell outside every table is an outline of its own.
    let mut boxes = half_page_table(2, 2);
    boxes.push(placed(BoxKind::Cell, [100.0, 600.0, 300.0, 700.0]));
    let ps = plans(&with_pages(Vec::new(), vec![page(1, boxes)]));
    assert_eq!(count(&ps[0], ShapeKind::Outline), 2);
}
