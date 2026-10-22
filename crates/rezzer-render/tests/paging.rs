// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Text paging: a page that starts inside a section repeats the section's heading and the
//! parameter group title it stands under, marked continued (spec 8.7).

use rezzer_model::{
    Anchor, Block, Certainty, Completeness, Confidence, Detection, Document, FormatId, Formula,
    FormulaKind, Inline, LayoutKind, Parameter, Role, SectionKind, SourceInfo, TableLayout, Value,
};
use rezzer_render::{OutputSection, RenderOptions, paginate, render};

/// A page without the lines a continued page begins with (`... (continued)` and a blank).
fn without_lead(page: &str) -> &str {
    let mut rest = page;
    while let Some((line, after)) = rest.split_once("\n\n") {
        if line.contains('\n') || !line.ends_with(" (continued)") {
            break;
        }
        rest = after;
    }
    rest
}

/// The pages joined without the lines a continued page begins with.
fn unpaged(pages: &[String]) -> String {
    pages
        .iter()
        .enumerate()
        .map(|(i, p)| if i == 0 { p.as_str() } else { without_lead(p) })
        .collect()
}

fn workbook() -> Document {
    let mut d = Document::new(SourceInfo {
        path: None,
        name: "calc.xlsx".into(),
        sha256: "00".into(),
        size: 1,
        format: FormatId::Xlsx,
        detection: Detection {
            confidence: Confidence::Certain,
            evidence: "zip with xl/workbook.xml".into(),
        },
        modified: None,
        encoding: None,
        reader: None,
    });
    d.body = vec![Block::Section {
        kind: SectionKind::Sheet,
        title: Some("Lining".into()),
        anchor: Some(Anchor("Lining".into())),
        blocks: vec![Block::Paragraph {
            inlines: vec![Inline::plain("values")],
            role: None,
            anchor: None,
        }],
    }];
    d
}

/// A sheet region over `range` with no rows of its own: what the Logic groups follow.
fn region(range: &str) -> Block {
    Block::Table {
        caption: None,
        header_rows: 0,
        role: None,
        rows: Vec::new(),
        alignments: Vec::new(),
        anchor: Some(Anchor(format!("Lining!{range}"))),
        layout: Some(Box::new(TableLayout {
            kind: LayoutKind::KeyValue,
            key: format!("Lining!{range}"),
            range: range.into(),
            data: None,
            header_rows: Vec::new(),
            header_paths: Vec::new(),
            label_cols: 0,
            total_columns: Vec::new(),
            certainty: Certainty::Rule,
            found_by: Vec::new(),
            unsure: Vec::new(),
            rows: Vec::new(),
            hidden_header: false,
        })),
    }
}

fn param(row: u32, path: &[&str]) -> Parameter {
    Parameter {
        anchor: Anchor(format!("Lining!B{row}")),
        name: None,
        label: Some(path.join(" / ")),
        value: Some(Value::number(f64::from(row), None)),
        unit: None,
        unit_source: None,
        read_by: 1,
        path: path.iter().map(|p| (*p).to_string()).collect(),
    }
}

fn logic_only() -> RenderOptions {
    RenderOptions {
        sections: Some(vec![OutputSection::Logic]),
        generated_at: "2026-10-20T12:00:00Z".into(),
        rezzer_version: "0.1.0".into(),
        ..RenderOptions::default()
    }
}

#[test]
fn split_section_repeats_title() {
    // A sheet part with three parameter groups and a long formula table, paginated small:
    // every page after the first starts inside `### Lining` and says so.
    let mut d = workbook();
    if let Block::Section { blocks, .. } = &mut d.body[0] {
        for range in ["A10:B15", "A20:B25", "A30:B35"] {
            blocks.push(Block::Heading {
                level: 3,
                inlines: vec![Inline::plain("Lining")],
                anchor: Some(Anchor(format!("Lining!{range}"))),
            });
            blocks.push(region(range));
        }
    }
    let mut params = Vec::new();
    for (g, name) in ["Loads", "Geometry", "Material"].iter().enumerate() {
        for i in 0..6u32 {
            let row = 10 * (g as u32 + 1) + i;
            params.push(param(row, &["Lining", name, &format!("value {i}")]));
        }
    }
    d.facets.parameters = params;
    d.facets.formulas = (1..=12u32)
        .map(|r| Formula {
            anchor: Anchor(format!("Lining!D{r}")),
            expression: format!("=B{}*2", 10 + r),
            dialect: "excel".into(),
            references: vec![Anchor(format!("Lining!B{}", 10 + r))],
            cached: Some(Value::number(1.0, None)),
            role: Some(Role::Intermediate),
            pattern: None,
            latex: None,
            note: None,
            kind: Some(FormulaKind::Cell),
            range: None,
            original: None,
        })
        .collect();
    let md = render(&d, Completeness::Complete, &logic_only());
    let start = md.find("### Lining\n").expect("the sheet's part");
    assert!(
        md[start..].contains("Parameters in Lining / Loads:"),
        "{md}"
    );
    let pages = paginate(&md, 120);
    assert!(pages.len() > 2, "{pages:#?}");
    assert_eq!(unpaged(&pages), md);
    let inside: Vec<&String> = pages
        .iter()
        .skip(1)
        .filter(|p| md.find(without_lead(p)).is_some_and(|at| at > start))
        .collect();
    assert!(inside.len() > 1, "{pages:#?}");
    for page in inside {
        assert!(page.starts_with("### Lining (continued)\n\n"), "{page}");
    }
}

#[test]
fn page_inside_a_group_repeats_its_title() {
    let filler = "word ".repeat(30);
    let md = format!(
        "# T\n\n## Logic\n\n### Lining\n\n#### Parameters\n\nParameters in Lining / Loads:\n\n| Cell | Label |\n| --- | --- |\n| B5 | Dead |\n\n{filler}\n\n{filler}\n\n#### Formulas\n\n{filler}\n"
    );
    let pages = paginate(&md, 40);
    assert_eq!(unpaged(&pages), md);
    // The pages that start with the filler stand in the group and under `### Lining`.
    let group: Vec<&String> = pages
        .iter()
        .filter(|p| without_lead(p).starts_with("word"))
        .collect();
    assert!(!group.is_empty(), "{pages:#?}");
    let in_group =
        format!("### Lining (continued)\n\nParameters in Lining / Loads: (continued)\n\n{filler}");
    assert!(group[0].starts_with(&in_group), "{pages:#?}");
    // A `####` heading ends the group.
    let formulas = pages
        .iter()
        .find(|p| without_lead(p).starts_with("#### Formulas"))
        .expect("a page at the Formulas heading");
    assert!(
        formulas.starts_with("### Lining (continued)\n\n#### Formulas"),
        "{pages:#?}"
    );
}

#[test]
fn page_outside_logic_repeats_nothing() {
    let filler = "word ".repeat(30);
    // Outside `## Logic` paging is as before: no lead, and `Parameters in` is a plain
    // paragraph, never a group title.
    let md = format!(
        "# T\n\n## Sheet\n\nParameters in a box:\n\n{filler}\n\n{filler}\n\n### Part\n\n{filler}\n\n{filler}\n"
    );
    let pages = paginate(&md, 40);
    assert!(pages.len() > 2, "{pages:#?}");
    assert_eq!(pages.concat(), md);
    for page in &pages {
        assert!(!page.contains(" (continued)"), "{pages:#?}");
    }
}

#[test]
fn page_at_a_section_start_in_logic_has_no_lead() {
    let filler = "word ".repeat(30);
    let md = format!("# T\n\n## Logic\n\n### A\n\n{filler}\n\n{filler}\n\n### B\n\n{filler}\n");
    let pages = paginate(&md, 40);
    assert_eq!(unpaged(&pages), md);
    let b = pages
        .iter()
        .find(|p| without_lead(p).starts_with("### B"))
        .expect("a page at ### B");
    assert!(b.starts_with("### B\n"), "{pages:#?}");
    assert!(
        pages.iter().any(|p| p.starts_with("### A (continued)\n\n")),
        "{pages:#?}"
    );
}

#[test]
fn untitled_table_after_a_group_ends_the_group() {
    let filler = "word ".repeat(30);
    // A table with no title line before it starts a group without a title, so a page after
    // it does not repeat the title of the group before.
    let md = format!(
        "# T\n\n## Logic\n\n### S\n\n#### Parameters\n\nParameters in Loads:\n\n| Cell |\n| --- |\n| B5 |\n\n| Cell |\n| --- |\n| H9 |\n\n{filler}\n\n{filler}\n"
    );
    let pages = paginate(&md, 40);
    assert_eq!(unpaged(&pages), md);
    let page = pages
        .iter()
        .find(|p| without_lead(p).starts_with("word"))
        .expect("a page at the filler");
    assert!(page.starts_with("### S (continued)\n\nword"), "{pages:#?}");
}
