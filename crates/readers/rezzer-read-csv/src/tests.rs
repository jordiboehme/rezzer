// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tests of the CSV reader.

use std::cell::Cell as Count;

use sheetlayout::{Certainty, Kind, Layout, Rect, Region};

use super::*;

/// A table region with header row `header` (0-based) over the rows to `bottom`.
fn table(header: u32, bottom: u32) -> Region {
    let rect = Rect {
        top: header,
        left: 0,
        bottom,
        right: 1,
    };
    Region {
        kind: Kind::Table,
        rect,
        data: Some(Rect {
            top: header + 1,
            ..rect
        }),
        captions: Vec::new(),
        caption_heading: None,
        notes: Vec::new(),
        heading: None,
        above_header: Vec::new(),
        header_rows: vec![header],
        body: Some(rect),
        header_paths: vec!["a".into(), "b".into()],
        label_cols: 0,
        total_cols: Vec::new(),
        certainty: Certainty::Rule,
        evidence: Vec::new(),
        unsure: Vec::new(),
        rows: Vec::new(),
        hidden_header: false,
        rest: false,
    }
}

#[test]
fn a_cut_region_analysis_gives_a_loss_per_region() {
    // Two tables: rows 0 to 5000, a blank line, then rows 5002 to 5004.
    let mut text = String::from("a,b\n");
    let mut starts = vec![(0u32, 0usize)];
    for i in 1..=5_000u32 {
        starts.push((i, text.len()));
        text.push_str(&format!("{i},{i}\n"));
    }
    text.push('\n');
    for (k, line) in ["c,d\n", "1,2\n", "3,4\n"].iter().enumerate() {
        starts.push((5_002 + k as u32, text.len()));
        text.push_str(line);
    }
    let dialect = csvsniff::sniff(&text).unwrap();
    let layout = Layout {
        regions: vec![table(0, 5_000), table(5_002, 5_004)],
        issues: Vec::new(),
    };
    let part = Records {
        text: &text,
        dialect: &dialect,
        starts: &starts,
        filled: &[],
        widest: 2,
    };
    // The first analysis is cut inside, the second region is not started.
    let calls = Count::new(0);
    let stop = || {
        calls.set(calls.get() + 1);
        calls.get() > 1
    };
    let found = region_facts(&part, &layout, "data", &stop).unwrap();
    assert!(found.tables.is_empty());
    let losses: Vec<(&str, &str)> = found
        .losses
        .iter()
        .map(|l| (l.location.as_str(), l.what.as_str()))
        .collect();
    assert_eq!(
        losses,
        [
            (
                "data!A1:B5001",
                "time limit reached while the column facts of the table A1:B5001 were counted; it has no Columns table"
            ),
            (
                "data",
                "time limit reached before the column facts were counted; no Columns table for the tables A5003:B5005"
            ),
        ]
    );
    assert!(
        found
            .losses
            .iter()
            .all(|l| l.why == LossReason::Uncertain && l.severity == Severity::Info)
    );
    // Without a stop each region has its table, the second one from its own rows.
    let found = region_facts(&part, &layout, "data", &|| false).unwrap();
    assert!(found.losses.is_empty());
    let captions: Vec<Option<&str>> = found
        .tables
        .iter()
        .map(|t| match t {
            Block::Table { caption, .. } => caption.as_deref(),
            _ => None,
        })
        .collect();
    assert_eq!(
        captions,
        [Some("Columns A1:B5001"), Some("Columns A5003:B5005")]
    );
    let Block::Table { rows, .. } = &found.tables[1] else {
        panic!()
    };
    assert_eq!(rows.len(), 3);
}
