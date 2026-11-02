// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Print-run forms: a form template with a data layer of padded fixed-pitch lines, built in
//! code with invented values only.

#[path = "../common/mod.rs"]
mod common;

mod boxes;
mod cells;
mod dates;
mod footnotes;
mod grids;
mod headings;
mod marked;
mod marks;
mod padded;
mod pay_slip;
mod rows;
mod sections;
mod text;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use dates::*;
use grids::*;
use headings::*;
use marked::*;
use marks::*;
use padded::*;
use pay_slip::*;
use sections::*;

use common::{DataLine, FormPage, all_text, boxed, data_line, in_order, label, mark, print_form};
use rezzer_model::{Block, Completeness, Inline, LossReason, ReadOutcome, Severity, plain_text};

/// The allocator of the Rezzer binary, so the reads run as they run in it.
#[global_allocator]
static ALLOCATOR: rezzer_limits::Allocator = rezzer_limits::Allocator;

/// `bytes` read with the default options and limits. Every read of this file also checks
/// that no placeholder of Rezzer reaches the Markdown or the JSON.
fn read(bytes: &[u8]) -> ReadOutcome {
    let out = common::read(bytes);
    assert_no_placeholder(&out);
    out
}

/// Asserts that neither the rendered Markdown nor the JSON of the model of `out` holds a
/// noncharacter of U+FDD0 to U+FDEF: those are Rezzer's own placeholders while a page is read.
fn assert_no_placeholder(out: &ReadOutcome) {
    let md = rezzer_render::render(
        &out.document,
        out.completeness,
        &rezzer_render::RenderOptions::default(),
    );
    // JSON writes the noncharacters as they are, not escaped.
    let json = serde_json::to_string(&out.document).expect("json");
    let placeholder = |c: char| ('\u{FDD0}'..='\u{FDEF}').contains(&c);
    assert!(
        !md.contains(placeholder),
        "a placeholder in the Markdown: {md}"
    );
    assert!(!json.contains(placeholder), "a placeholder in the JSON");
}

/// The `Datum` of the header field grid, a label over its value in a row of fields, is the
/// document date: the pairs of the grid say it.
fn assert_grid_date(out: &ReadOutcome) {
    let found = out.document.properties.document_date.as_ref();
    assert_eq!(
        found.map(|d| (d.date.as_str(), d.from.as_str())),
        Some(("2026-10-01", "date label \"Datum\" on page 1"))
    );
}

/// The read is complete and names no loss.
fn assert_no_loss(out: &ReadOutcome) {
    assert_eq!(
        out.completeness,
        Completeness::Complete,
        "{:?}",
        out.document.losses
    );
    assert!(out.document.losses.is_empty(), "{:?}", out.document.losses);
}

/// The read is complete and names no loss but the one for the date in the head that nothing
/// marks as the document date (the `Eintritt` field of the boxed fields).
fn assert_no_loss_but_the_head_date(out: &ReadOutcome) {
    assert_eq!(
        out.completeness,
        Completeness::Complete,
        "{:?}",
        out.document.losses
    );
    let what: Vec<&str> = out
        .document
        .losses
        .iter()
        .map(|l| l.what.as_str())
        .collect();
    assert_eq!(what, [ONE_DATE], "{:?}", out.document.losses);
}

/// The golden file of the synthetic print-run form: its Markdown and its JSON are kept as
/// snapshots, so a later change of the reader shows in them. The facts next to them hold
/// whatever the snapshots say; [`read`] also checks that no placeholder reaches either.
///
/// Known wrong order, pinned on purpose until the region engine reads it: the labels over the
/// boxed amount and the boxed date in one column come before both values
/// (`Betrag`, `Zahltag`, `9876,54`, `28.02.2026`). Wanted: each label with its own value,
/// `Betrag` 9876,54 then `Zahltag` 28.02.2026. The snapshot changes when that is fixed.
#[test]
fn synthetic_print_form_golden() {
    let out = read(&common::synthetic_print_form());
    let md = common::markdown(&out);
    let json = serde_json::to_string_pretty(&out.document).expect("json");
    insta::with_settings!({snapshot_path => "../snapshots", prepend_module_to_snapshot => false}, {
        insta::assert_snapshot!("synthetic_print_form_md", md);
        insta::assert_snapshot!("synthetic_print_form_json", json);
    });

    assert_eq!(out.completeness, Completeness::Complete);
    let props = &out.document.properties;
    assert_eq!(
        props.title_from.as_deref(),
        Some("largest heading in the top third of page 1")
    );
    assert_eq!(
        props
            .document_date
            .as_ref()
            .map(|d| (d.date.as_str(), d.from.as_str())),
        Some(("2026-10-01", "date label \"Datum\" on page 1"))
    );
    let pages = common::synthetic_print_form_pages();
    let mut separators = 0;
    for (n, page) in pages.iter().enumerate() {
        let blocks = form_page_blocks(&out, n + 1);
        // Every token of every data line stands in the page's text once per drawing. The
        // boxed amount and the boxed date are read with their drawn separators; tokens the
        // template also holds are not counted.
        let mut want: HashMap<String, usize> = HashMap::new();
        for l in &page.lines {
            for t in l.text.split_whitespace() {
                let t = match t.len() {
                    6 if t.chars().all(|c| c.is_ascii_digit()) => {
                        format!("{},{}", &t[..4], &t[4..])
                    }
                    8 if t.chars().all(|c| c.is_ascii_digit()) => {
                        format!("{}.{}.{}", &t[..2], &t[2..4], &t[4..])
                    }
                    _ => t.to_string(),
                };
                *want.entry(t).or_default() += 1;
            }
        }
        // The text of the page without its notes and its page marker.
        let kept: Vec<Block> = blocks
            .iter()
            .filter(|b| !matches!(b, Block::Paragraph { role: Some(_), .. }))
            .cloned()
            .collect();
        let text = common::text(&kept);
        let mut got: HashMap<&str, usize> = HashMap::new();
        for t in text.split(|c: char| c.is_whitespace() || c == '|') {
            *got.entry(t).or_default() += 1;
        }
        let template = template_words(&page.template);
        let mut checked = 0;
        for (t, k) in &want {
            if template.contains(t) {
                continue;
            }
            checked += 1;
            assert_eq!(
                got.get(t.as_str()).copied().unwrap_or(0),
                *k,
                "page {}: {t:?} in {text}",
                n + 1
            );
        }
        assert!(checked >= 25, "page {}: {checked} tokens checked", n + 1);
        // One note for the separators, and one for each kind of pairs, on each page.
        let notes: Vec<String> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::Paragraph {
                    inlines,
                    role: Some(r),
                    ..
                } if r == "found-by" => Some(plain_text(inlines)),
                _ => None,
            })
            .collect();
        separators += notes.iter().filter(|s| s.contains("separator")).count();
        assert_eq!(
            notes.iter().filter(|s| s.contains("separator")).count(),
            1,
            "{notes:?}"
        );
        for note in &notes {
            assert_eq!(
                notes.iter().filter(|s| *s == note).count(),
                1,
                "page {}: {notes:?}",
                n + 1
            );
        }
        assert!(notes.iter().any(|s| s == boxes::BOX_NOTE), "{notes:?}");
    }
    assert_eq!(separators, 2);
}

/// The words the text objects of the template `content` show: the strings of each text
/// object joined (a label is shown in two kerned parts), cut at white space.
fn template_words(content: &str) -> Vec<String> {
    let mut words = Vec::new();
    for object in content.split("BT ").skip(1) {
        let object = object.split(" ET").next().unwrap_or("");
        let mut shown = String::new();
        let mut rest = object;
        while let Some(open) = rest.find('(') {
            let close = rest[open..].find(')').map_or(rest.len(), |c| open + c);
            shown += &rest[open + 1..close];
            rest = &rest[(close + 1).min(rest.len())..];
        }
        words.extend(shown.split_whitespace().map(str::to_string));
    }
    words
}

/// The blocks of page `n` of `out`.
fn form_page_blocks(out: &ReadOutcome, n: usize) -> &[Block] {
    let want = rezzer_model::Anchor(format!("p{n}"));
    out.document
        .body
        .iter()
        .find_map(|b| match b {
            Block::Section {
                kind: rezzer_model::SectionKind::Page,
                anchor: Some(a),
                blocks,
                ..
            } if *a == want => Some(blocks.as_slice()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no page {n}"))
}
