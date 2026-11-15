// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The note levels: `full` writes every note, `brief` leaves out the notes that only say how
//! something was read and counts them under `## Not read`, `none` writes no notes and says
//! so in one line. The frontmatter is the same at every level.

use rezzer_model::{
    Anchor, Block, Cell, Certainty, Completeness, Confidence, Detection, Document, Evidence,
    FormatId, Inline, LayoutKind, Loss, LossReason, ReaderInfo, SectionKind, Severity, SourceInfo,
    TableLayout,
};
use rezzer_render::{NoteLevel, OutputSection, RenderOptions, render, render_output};

fn source(name: &str, format: FormatId) -> SourceInfo {
    SourceInfo {
        path: Some(format!("/data/{name}")),
        name: name.into(),
        sha256: "abc123".into(),
        size: 42,
        format,
        detection: Detection {
            confidence: Confidence::Certain,
            evidence: "content".into(),
        },
        modified: None,
        created: None,
        downloaded_from: None,
        file_tags: Vec::new(),
        encoding: None,
        reader: Some(ReaderInfo {
            id: "test".into(),
            version: "0.1.0".into(),
        }),
    }
}

fn opts(notes: NoteLevel) -> RenderOptions {
    RenderOptions {
        generated_at: "2026-10-20T12:00:00Z".into(),
        rezzer_version: "0.1.0".into(),
        notes,
        ..RenderOptions::default()
    }
}

fn md(d: &Document, notes: NoteLevel) -> String {
    render(d, Completeness::Complete, &opts(notes))
}

fn txt(text: &str) -> Inline {
    Inline::plain(text)
}

fn frontmatter(md: &str) -> &str {
    let end = md[4..].find("\n---\n").expect("frontmatter end") + 4;
    &md[..end]
}

/// The `## Not read` section, or "" when there is none.
fn not_read(md: &str) -> &str {
    match md.find("## Not read") {
        Some(at) => {
            let rest = &md[at..];
            &rest[..rest[1..].find("\n## ").map_or(rest.len(), |e| e + 1)]
        }
        None => "",
    }
}

/// A cell of sheet `Ledger` at an A1 address.
fn at(text: &str, a1: &str) -> Cell {
    let mut c = Cell::new(vec![txt(text)]);
    c.anchor = Some(Anchor(format!("Ledger!{a1}")));
    c
}

fn sheet_table(found_by: Vec<Evidence>) -> Block {
    Block::Table {
        caption: None,
        header_rows: 1,
        role: None,
        rows: vec![
            vec![at("Item", "A1"), at("Amount", "B1")],
            vec![at("Pens", "A2"), at("4", "B2")],
            vec![at("Ink", "A3"), at("7", "B3")],
        ],
        alignments: Vec::new(),
        anchor: Some(Anchor("Ledger!A1:B3".into())),
        layout: Some(Box::new(TableLayout {
            kind: LayoutKind::Table,
            key: "Ledger!A1".into(),
            range: "A1:B3".into(),
            data: Some("A2:B3".into()),
            header_rows: vec![1],
            header_paths: vec!["Item".into(), "Amount".into()],
            label_cols: 0,
            total_columns: Vec::new(),
            certainty: Certainty::Rule,
            found_by,
            unsure: Vec::new(),
            rows: Vec::new(),
            hidden_header: false,
        })),
    }
}

fn book(blocks: Vec<Block>) -> Document {
    let mut d = Document::new(source("book.xlsx", FormatId::Xlsx));
    d.body = vec![Block::Section {
        kind: SectionKind::Sheet,
        title: Some("Ledger".into()),
        anchor: Some(Anchor("Ledger".into())),
        blocks,
    }];
    d
}

fn excel_table() -> Evidence {
    Evidence::ListObject {
        name: "Stock".into(),
    }
}

fn gaps() -> Evidence {
    Evidence::Rule {
        id: "G".into(),
        detail: "empty-row gaps".into(),
    }
}

#[test]
fn full_keeps_the_rule_words_of_a_sheet_table() {
    let d = book(vec![sheet_table(vec![excel_table(), gaps()])]);
    let full = md(&d, NoteLevel::Full);
    assert!(
        full.contains(
            "Table A1:B3 (data A2:B3), header row 1, found by: Excel table Stock; rules: \
             empty-row gaps.\n"
        ),
        "{full}"
    );
    assert_eq!(not_read(&full), "", "{full}");
    assert_eq!(
        full,
        render(
            &d,
            Completeness::Complete,
            &RenderOptions {
                generated_at: "2026-10-20T12:00:00Z".into(),
                rezzer_version: "0.1.0".into(),
                ..RenderOptions::default()
            }
        ),
        "full is the render default"
    );
}

fn rule(id: &str, detail: &str) -> Evidence {
    Evidence::Rule {
        id: id.into(),
        detail: detail.into(),
    }
}

/// The facts of the file and the rules that say where a part is stay in the layout line;
/// the rules that only say the method go. That is no gap, so there is no count line and no
/// `## Not read` heading, and the output is not longer than at `full`.
#[test]
fn brief_keeps_the_facts_and_leaves_out_the_method_without_a_line() {
    let facts = vec![
        excel_table(),
        Evidence::Pivot {
            name: "Summary".into(),
        },
        Evidence::DefinedName {
            name: "Rates".into(),
        },
        Evidence::DatabaseRange {
            name: "Orders".into(),
        },
        Evidence::Autofilter,
        Evidence::PrintTitles,
        Evidence::FrozenPanes,
    ];
    let mut evidence = facts.clone();
    evidence.extend([
        gaps(),
        rule(
            "G",
            "empty-row and empty-column gaps, apart from a block it touches only at a corner",
        ),
        rule("H1", "header row 1 by type change"),
        rule("H2", "bold header row 1"),
        rule("H3", "all-text header row 1, every cell bold"),
        rule("L", "label column B"),
        rule("K", "column C is not a unit column"),
        rule("T", "caption rows 1 to 2"),
        rule("T2", "title in row 27"),
        rule("R1", "section heading in row 5"),
        rule("C", "continued in rows 12 to 40"),
        rule("N", "notes in row 9"),
        rule("G", "rest of the sheet after the cut depth limit"),
        rule("Z", "words of a rule not known yet"),
    ]);
    let d = book(vec![sheet_table(evidence)]);
    let full = md(&d, NoteLevel::Full);
    let brief = md(&d, NoteLevel::Brief);
    let line = "Table A1:B3 (data A2:B3), header row 1, found by: Excel table Stock, pivot table \
                Summary, defined name Rates, database range Orders, filter range, print titles, \
                frozen panes; rules: caption rows 1 to 2, title in row 27, section heading in \
                row 5, continued in rows 12 to 40, notes in row 9, rest of the sheet after the \
                cut depth limit, words of a rule not known yet.\n";
    assert!(brief.contains(line), "{brief}");
    assert!(full.contains("empty-row gaps"), "{full}");
    for method in [
        "gaps",
        "type change",
        "bold header",
        "all-text",
        "label column",
        "unit column",
    ] {
        assert!(!brief.contains(method), "{method}: {brief}");
    }
    assert!(
        !brief.contains("## Not read"),
        "no count line for no gap: {brief}"
    );
    assert!(brief.len() < full.len());
    // The facts stay at `none` too.
    let only_facts = book(vec![sheet_table(facts)]);
    for level in NoteLevel::ALL {
        assert!(
            md(&only_facts, level).contains(
                "found by: Excel table Stock, pivot table Summary, defined name Rates, database \
                 range Orders, filter range, print titles, frozen panes.\n"
            ),
            "{level:?}"
        );
    }
}

/// A file whose only left-out text is method words gets no line and is never longer than
/// at `full`: the words go, nothing comes.
#[test]
fn method_words_alone_add_nothing() {
    let d = book(vec![sheet_table(vec![gaps()]), sheet_table(Vec::new())]);
    let full = md(&d, NoteLevel::Full);
    let brief = md(&d, NoteLevel::Brief);
    assert!(!brief.contains("## Not read"), "{brief}");
    assert!(brief.len() < full.len(), "{brief}");
    assert_eq!(
        full.len() - brief.len(),
        ", found by rules: empty-row gaps".len()
    );
}

#[test]
fn brief_says_a_header_not_found_where_only_the_rules_said_it() {
    let mut t = sheet_table(vec![Evidence::Rule {
        id: "H0".into(),
        detail: "no header found".into(),
    }]);
    if let Block::Table {
        layout: Some(l),
        header_rows,
        ..
    } = &mut t
    {
        l.header_rows.clear();
        l.header_paths.clear();
        *header_rows = 0;
    }
    let d = book(vec![t]);
    let full = md(&d, NoteLevel::Full);
    assert!(
        full.contains(", found by rules: no header found.\n"),
        "{full}"
    );
    let brief = md(&d, NoteLevel::Brief);
    assert!(
        brief.contains("Table A1:B3 (data A2:B3), no header found.\n"),
        "{brief}"
    );
}

#[test]
fn the_frontmatter_is_the_same_at_every_level() {
    let mut d = book(vec![sheet_table(vec![excel_table()])]);
    d.losses.push(Loss {
        location: "Ledger!A1".into(),
        what: "a part that was not read".into(),
        why: LossReason::Unsupported,
        severity: Severity::Warning,
    });
    let full = md(&d, NoteLevel::Full);
    for level in NoteLevel::ALL {
        let out = md(&d, level);
        assert_eq!(frontmatter(&out), frontmatter(&full), "{level:?}");
        assert!(out.contains("\nlosses: 1\n"), "{out}");
    }
}

/// The grid doubt is left out only where the grid line is written: without the content
/// it is a row again, and no row is counted.
#[test]
fn the_left_out_line_follows_the_chosen_sections() {
    let mut d = book(vec![grid_table("A1:B1", &["no header found"])]);
    d.losses = vec![loss(
        "Ledger!A1:B1",
        "no header found",
        LossReason::Uncertain,
        Severity::Info,
    )];
    let only_not_read = RenderOptions {
        sections: Some(vec![OutputSection::NotRead]),
        ..opts(NoteLevel::Brief)
    };
    let out = render(&d, Completeness::Complete, &only_not_read);
    assert!(out.contains("| Ledger!A1:B1 | no header found |"), "{out}");
    assert!(!out.contains("not shown"), "{out}");
    let both = RenderOptions {
        sections: Some(vec![OutputSection::Content, OutputSection::NotRead]),
        ..opts(NoteLevel::Brief)
    };
    let out = render(&d, Completeness::Complete, &both);
    assert!(
        out.contains("1 note on how the file was read is not shown. `--notes full` shows it."),
        "{out}"
    );
}

#[test]
fn a_split_document_says_the_left_out_notes_in_its_main_file() {
    let rows: Vec<Vec<Cell>> = std::iter::once(vec![at("Item", "A1"), at("Amount", "B1")])
        .chain((2..=400).map(|r| {
            vec![
                at(
                    &format!("Thing number {r} with some words"),
                    &format!("A{r}"),
                ),
                at(&r.to_string(), &format!("B{r}")),
            ]
        }))
        .collect();
    let mut t = sheet_table(vec![gaps()]);
    if let Block::Table {
        rows: r,
        layout: Some(l),
        anchor,
        ..
    } = &mut t
    {
        *r = rows;
        l.range = "A1:B400".into();
        l.data = Some("A2:B400".into());
        *anchor = Some(Anchor("Ledger!A1:B400".into()));
    }
    let mut d = book(vec![t]);
    d.losses = vec![loss(
        "p1:b2",
        "the region at p1:b2 is kept as laid out, line by line: no region type has clear \
         evidence for it",
        LossReason::Uncertain,
        Severity::Info,
    )];
    let out = render_output(
        &d,
        Completeness::Complete,
        &opts(NoteLevel::Brief),
        2500,
        "book.xlsx.md",
        "book.xlsx.parts",
    );
    assert!(!out.parts.is_empty(), "the document is split");
    assert!(
        out.main
            .contains("1 note on how the file was read is not shown"),
        "{}",
        out.main
    );
    for part in &out.parts {
        assert!(
            !part.markdown.contains("empty-row gaps"),
            "{}",
            part.markdown
        );
    }
}

fn loss(location: &str, what: &str, why: LossReason, severity: Severity) -> Loss {
    Loss {
        location: location.into(),
        what: what.into(),
        why,
        severity,
    }
}

/// A PDF page with a region kept as laid out (left out in brief) beside notes that touch a
/// value or say that something is missing (kept).
fn paged() -> Document {
    let mut d = Document::new(source("form.pdf", FormatId::Pdf));
    d.body = vec![Block::paragraph(vec![txt("Total 12,00")])];
    let laid_out = "the region at p1:b2 is kept as laid out, line by line: no region type has \
                    clear evidence for it";
    d.losses = vec![
        loss("p1:b2", laid_out, LossReason::Uncertain, Severity::Info),
        loss(
            "p1:b3",
            "the region at p1:b3 is kept as laid out, line by line: it reads as heading or \
             as paragraph about equally (410 and 400)",
            LossReason::Uncertain,
            Severity::Info,
        ),
        loss(
            "p1",
            "2 lines of a table on page 1 are kept as lines of the rows above them: the page \
             does not show whether they go on those rows or are rows of their own",
            LossReason::Uncertain,
            Severity::Info,
        ),
        loss(
            "p1",
            "Possible table on page 1 not rebuilt (reason: most of its cells are empty)",
            LossReason::Uncertain,
            Severity::Info,
        ),
        // A limit left the region whole: something was not done.
        loss(
            "p1:b4",
            &laid_out.replace("b2", "b4"),
            LossReason::Limit,
            Severity::Info,
        ),
        // The same words at a higher severity are no plain note on the reading.
        loss(
            "p1:b5",
            &laid_out.replace("b2", "b5"),
            LossReason::Uncertain,
            Severity::Warning,
        ),
        loss(
            "p2",
            "3 glyphs without a Unicode mapping on page 2",
            LossReason::Unsupported,
            Severity::Info,
        ),
    ];
    d
}

#[test]
fn brief_leaves_out_regions_kept_as_laid_out_and_keeps_the_rest() {
    let d = paged();
    let full = md(&d, NoteLevel::Full);
    assert_eq!(full.matches(" is kept as laid out, ").count(), 4, "{full}");
    let brief = md(&d, NoteLevel::Brief);
    let rows = not_read(&brief);
    assert!(!rows.contains("p1:b2 is kept"), "{rows}");
    assert!(
        !rows.contains("p1:b3 is kept"),
        "a kind decided with doubt: {rows}"
    );
    for kept in [
        "2 lines of a table on page 1 are kept as lines of the rows above them",
        "Possible table on page 1 not rebuilt",
        "the region at p1:b4 is kept as laid out",
        "the region at p1:b5 is kept as laid out",
        "3 glyphs without a Unicode mapping on page 2",
    ] {
        assert!(rows.contains(kept), "{kept}: {rows}");
    }
    assert!(
        rows.trim_end().ends_with(
            "\n\n2 notes on how the file was read are not shown. `--notes full` shows them."
        ),
        "{rows}"
    );
    assert_eq!(frontmatter(&brief), frontmatter(&full));
    assert!(brief.contains("\nlosses: 7\n"), "{brief}");
}

/// A sheet region written as a grid, unsure of its header for `reasons`.
fn grid_table(range: &str, reasons: &[&str]) -> Block {
    Block::Table {
        caption: None,
        header_rows: 0,
        role: None,
        rows: vec![vec![at("Pens", "A1"), at("4", "B1")]],
        alignments: Vec::new(),
        anchor: Some(Anchor(format!("Ledger!{range}"))),
        layout: Some(Box::new(TableLayout {
            kind: LayoutKind::Grid,
            key: "Ledger!A1".into(),
            range: range.into(),
            data: None,
            header_rows: Vec::new(),
            header_paths: Vec::new(),
            label_cols: 0,
            total_columns: Vec::new(),
            certainty: Certainty::Unsure,
            found_by: Vec::new(),
            unsure: reasons.iter().map(|r| r.to_string()).collect(),
            rows: Vec::new(),
            hidden_header: false,
        })),
    }
}

#[test]
fn brief_leaves_out_the_header_doubt_its_grid_line_already_says() {
    let mut d = book(vec![grid_table("A1:B1", &["no header found"])]);
    d.losses = vec![
        loss(
            "Ledger!A1:B1",
            "no header found",
            LossReason::Uncertain,
            Severity::Info,
        ),
        // No grid of the content says these.
        loss(
            "Ledger!D1:E1",
            "no header found",
            LossReason::Uncertain,
            Severity::Info,
        ),
        loss(
            "Ledger!A1:B1",
            "the region may hold more than one table",
            LossReason::Uncertain,
            Severity::Info,
        ),
    ];
    let full = md(&d, NoteLevel::Full);
    assert!(
        full.contains("| Ledger!A1:B1 | no header found |"),
        "{full}"
    );
    let brief = md(&d, NoteLevel::Brief);
    assert!(
        brief.contains("Grid A1:B1, header unsure: no header found.\n"),
        "the line says it: {brief}"
    );
    let rows = not_read(&brief);
    assert!(
        !rows.contains("| Ledger!A1:B1 | no header found |"),
        "{rows}"
    );
    assert!(
        rows.contains("| Ledger!D1:E1 | no header found |"),
        "{rows}"
    );
    assert!(
        rows.contains("the region may hold more than one table"),
        "{rows}"
    );
    assert!(
        rows.contains("\n\n1 note on how the file was read is not shown."),
        "{rows}"
    );
    // Without the content the grid line is not written, and the loss stays.
    let only = RenderOptions {
        sections: Some(vec![OutputSection::NotRead]),
        ..opts(NoteLevel::Brief)
    };
    let out = render(&d, Completeness::Complete, &only);
    assert!(out.contains("| Ledger!A1:B1 | no header found |"), "{out}");
}

fn found_by(text: &str) -> Block {
    Block::Paragraph {
        inlines: vec![txt(text)],
        role: Some(rezzer_model::FOUND_BY_ROLE.into()),
        anchor: None,
    }
}

#[test]
fn brief_leaves_out_found_by_notes_on_tables_pairs_and_footnotes() {
    let separators = "Found by: 2 decimal separators on this page are drawn marks, not text; \
                      they are written as \",\" as in 4 amounts written as text in this file.";
    let footnotes = format!(
        "2 footnotes at the bottom of page 1 are read from their legend, {}; the markers \
         above are linked to them",
        rezzer_model::FOOTNOTE_FOUND_BY
    );
    let mut d = Document::new(source("form.pdf", FormatId::Pdf));
    d.body = vec![
        found_by(separators),
        found_by(&footnotes),
        found_by(rezzer_model::BOX_FOUND_BY),
        Block::paragraph(vec![txt("Gross 1.234,56")]),
        found_by(rezzer_model::STRIP_FOUND_BY),
        found_by(rezzer_model::SIDE_FOUND_BY),
        Block::paragraph(vec![txt("Net 1.000,00")]),
    ];
    let full = md(&d, NoteLevel::Full);
    for text in [
        separators,
        &footnotes,
        rezzer_model::BOX_FOUND_BY,
        rezzer_model::STRIP_FOUND_BY,
        rezzer_model::SIDE_FOUND_BY,
    ] {
        assert!(full.contains(text), "{text}: {full}");
    }
    let brief = md(&d, NoteLevel::Brief);
    assert!(
        brief.contains(separators),
        "a value got a character: {brief}"
    );
    for text in [
        &footnotes[..],
        rezzer_model::BOX_FOUND_BY,
        rezzer_model::STRIP_FOUND_BY,
        rezzer_model::SIDE_FOUND_BY,
    ] {
        assert!(!brief.contains(text), "{text}: {brief}");
    }
    assert!(brief.contains("Gross 1.234,56\n\nNet 1.000,00"), "{brief}");
    assert!(
        !brief.contains("## Not read"),
        "an explanation is no gap, no line: {brief}"
    );
}

#[test]
fn none_writes_no_notes_and_says_so_in_one_line() {
    let d = paged();
    let full = md(&d, NoteLevel::Full);
    let none = md(&d, NoteLevel::None);
    assert_eq!(frontmatter(&none), frontmatter(&full));
    assert!(none.contains("Total 12,00"), "{none}");
    assert!(!none.contains("| Where |"), "{none}");
    assert_eq!(
        not_read(&none).trim_end(),
        "## Not read\n\nNotes are not shown. `--notes brief` or `--notes full` shows them.",
        "{none}"
    );
    // A document without any note has no such line.
    let mut plain = Document::new(source("plain.pdf", FormatId::Pdf));
    plain.body = vec![Block::paragraph(vec![txt("Total 12,00")])];
    assert!(!md(&plain, NoteLevel::None).contains("## Not read"));
}

/// A table of a page with a cell over two columns: the note on it says the covered cell is
/// empty, at every level but `none`.
#[test]
fn the_note_on_merged_cells_stays_but_at_none() {
    let mut wide = Cell::new(vec![txt("Fees")]);
    wide.col_span = 2;
    let mut d = Document::new(source("form.pdf", FormatId::Pdf));
    d.body = vec![Block::Table {
        caption: None,
        header_rows: 1,
        role: None,
        rows: vec![
            vec![Cell::new(vec![txt("Item")]), Cell::new(vec![txt("Amount")])],
            vec![wide],
            vec![Cell::new(vec![txt("Ink")]), Cell::new(vec![txt("7")])],
        ],
        alignments: Vec::new(),
        anchor: Some(Anchor("p1:t1".into())),
        layout: None,
    }];
    let merged = "Merged cells: `Fees` (row 2 column 1) over 2 columns.";
    assert!(md(&d, NoteLevel::Full).contains(merged));
    let brief = md(&d, NoteLevel::Brief);
    assert!(brief.contains(merged), "{brief}");
    assert!(!brief.contains("## Not read"), "nothing left out: {brief}");
    let none = md(&d, NoteLevel::None);
    assert!(!none.contains("Merged cells"), "{none}");
    assert!(none.contains("| Fees |  |"), "{none}");
    assert!(
        none.ends_with(
            "\n## Not read\n\nNotes are not shown. `--notes brief` or `--notes full` shows them.\n"
        ),
        "the note on merged cells is dropped, so the line says so: {none}"
    );
}

/// At `none` a dropped found-by note gets the line too, also the one on separators that
/// `brief` keeps; a document whose notes are all kept by nothing has no line.
#[test]
fn none_says_so_when_it_drops_a_found_by_note() {
    let mut d = Document::new(source("form.pdf", FormatId::Pdf));
    d.body = vec![
        found_by(
            "Found by: 2 decimal separators on this page are drawn marks, not text; they are \
             written as \",\" as in 4 amounts written as text in this file.",
        ),
        Block::paragraph(vec![txt("Gross 1.234,56")]),
    ];
    let none = md(&d, NoteLevel::None);
    assert!(!none.contains("Found by"), "{none}");
    assert!(none.contains("Notes are not shown."), "{none}");
    let mut plain = Document::new(source("form.pdf", FormatId::Pdf));
    plain.body = vec![Block::paragraph(vec![txt("Gross 1.234,56")])];
    assert!(!md(&plain, NoteLevel::None).contains("## Not read"));
    // A table with a merged cell whose note `none` would not write either: the cell is in
    // the header band of several rows, which names its cells over their columns.
    let mut wide = Cell::new(vec![txt("Fees")]);
    wide.col_span = 2;
    let mut banded = Document::new(source("form.pdf", FormatId::Pdf));
    banded.body = vec![Block::Table {
        caption: None,
        header_rows: 2,
        role: None,
        rows: vec![
            vec![wide],
            vec![Cell::new(vec![txt("Item")]), Cell::new(vec![txt("Amount")])],
            vec![Cell::new(vec![txt("Ink")]), Cell::new(vec![txt("7")])],
        ],
        alignments: Vec::new(),
        anchor: Some(Anchor("p1:t1".into())),
        layout: None,
    }];
    assert!(!md(&banded, NoteLevel::Full).contains("Merged cells"));
    assert!(!md(&banded, NoteLevel::None).contains("## Not read"));
}

/// Only the notes of a region kept for doubt about its kind go: no type is clear, two read
/// about equally, one reads but not clearly enough. A region whose kind its reader built
/// nothing from keeps its note (the ties of its values may be missing), and so does a note
/// that says the bold or italic text is marked in the JSON only.
#[test]
fn brief_keeps_laid_out_notes_that_say_something_is_missing() {
    let mut d = Document::new(source("form.pdf", FormatId::Pdf));
    d.body = vec![Block::paragraph(vec![txt("Total 12,00")])];
    let at = |b: u32, how: &str, why: &str| {
        loss(
            &format!("p1:b{b}"),
            &format!("the region at p1:b{b} is kept as laid out, {how}: {why}"),
            LossReason::Uncertain,
            Severity::Info,
        )
    };
    let gone = [
        at(1, "line by line", rezzer_model::LAID_OUT_NO_EVIDENCE),
        at(
            2,
            "as text in its columns",
            rezzer_model::LAID_OUT_NO_EVIDENCE,
        ),
        at(
            3,
            "line by line",
            "it reads as header band or as heading about equally (450 and 420 of 1000)",
        ),
        at(
            4,
            "line by line",
            "it reads as table (350 of 1000), but not clearly enough",
        ),
    ];
    let kept = [
        at(
            5,
            "as text in its columns",
            "no region type has clear evidence for it; its bold text is marked in the JSON only",
        ),
        at(
            6,
            "as text in its columns",
            "it reads as table (350 of 1000), but not clearly enough; its bold and italic text \
             is marked in the JSON only",
        ),
        at(
            7,
            "line by line",
            "it reads as table, but most of its cells are empty",
        ),
        at(
            8,
            "line by line",
            "it reads as totals, but no totals could be built from it",
        ),
    ];
    d.losses = gone.iter().chain(&kept).cloned().collect();
    let rows = not_read(&md(&d, NoteLevel::Brief)).to_string();
    for l in &gone {
        assert!(!rows.contains(&l.what), "{}: {rows}", l.what);
    }
    for l in &kept {
        assert!(rows.contains(&l.what), "{}: {rows}", l.what);
    }
    assert!(
        rows.contains("4 notes on how the file was read are not shown."),
        "{rows}"
    );
}

/// The footnote note of a page goes in `brief` only when all its footnotes are linked to
/// their markers; when one is not, the note says a link is missing, and it stays.
#[test]
fn brief_keeps_the_footnote_note_that_says_a_link_is_missing() {
    let how = rezzer_model::FOOTNOTE_FOUND_BY;
    let all_one = format!(
        "1 footnote at the bottom of page 1 is read from its legend, {how}{}",
        rezzer_model::FOOTNOTE_LINKED
    );
    let all = format!(
        "2 footnotes at the bottom of page 2 are read from their legend, {how}{}",
        rezzer_model::FOOTNOTES_LINKED
    );
    let none_linked = format!(
        "1 footnote at the bottom of page 3 is read from its legend, {how}; no marker above \
         is linked to it"
    );
    let some = format!(
        "3 footnotes at the bottom of page 4 are read from their legend, {how}; 2 of them are \
         linked to their markers above"
    );
    let mut d = Document::new(source("paper.pdf", FormatId::Pdf));
    d.body = vec![
        found_by(&all_one),
        found_by(&all),
        found_by(&none_linked),
        found_by(&some),
    ];
    let full = md(&d, NoteLevel::Full);
    let brief = md(&d, NoteLevel::Brief);
    for text in [&all_one, &all, &none_linked, &some] {
        assert!(full.contains(text.as_str()), "{text}");
    }
    assert!(!brief.contains(&all_one), "{brief}");
    assert!(!brief.contains(&all), "{brief}");
    assert!(brief.contains(&none_linked), "{brief}");
    assert!(brief.contains(&some), "{brief}");
}

/// Body rows of sheet `Ledger` with a cell over two columns in row 2.
fn merged_body() -> Vec<Vec<Cell>> {
    let mut wide = at("Pens and ink", "A2");
    wide.col_span = 2;
    vec![
        vec![at("Item", "A1"), at("Amount", "B1")],
        vec![wide],
        vec![at("Ink", "A3"), at("7", "B3")],
    ]
}

/// At `none` the note on merged body cells of a sheet table or grid is dropped and the line
/// says so; a table without merged cells, whose method words also go, gets no line: those
/// words are explanation, not a note.
#[test]
fn none_says_so_for_merged_cells_of_a_sheet_table_or_grid() {
    let mut table = sheet_table(vec![gaps()]);
    let mut grid = grid_table("A1:B3", &["no header found"]);
    for t in [&mut table, &mut grid] {
        if let Block::Table { rows, .. } = t {
            *rows = merged_body();
        }
    }
    for (what, block) in [("table", table), ("grid", grid)] {
        let d = book(vec![block]);
        let full = md(&d, NoteLevel::Full);
        assert!(full.contains("Merged cells: "), "{what}: {full}");
        let none = md(&d, NoteLevel::None);
        assert!(!none.contains("Merged cells"), "{what}: {none}");
        assert!(
            none.ends_with(
                "\n## Not read\n\nNotes are not shown. `--notes brief` or `--notes full` shows \
                 them.\n"
            ),
            "{what}: {none}"
        );
    }
    let plain = book(vec![sheet_table(vec![gaps()])]);
    assert!(md(&plain, NoteLevel::Full).contains("empty-row gaps"));
    let none = md(&plain, NoteLevel::None);
    assert!(!none.contains("empty-row gaps"), "{none}");
    assert!(!none.contains("## Not read"), "{none}");
}
