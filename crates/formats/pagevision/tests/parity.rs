// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Parity with the region bake-off code.
//!
//! The private tests are opt-in: they need `PAGEVISION_PRIVATE_EVAL` (the local private-eval
//! folder) and `--run-ignored only`. They find their pages through the numbers written there,
//! never through a page name in this file.

use std::path::{Path, PathBuf};

use pagevision::ink::{Mask, components, flatten, ink_mask};
use pagevision::rules::{Rect, RuleFixes, RuleKind, combine_c, ink_rules};
use pagevision::snap::snap;

fn private_eval() -> PathBuf {
    std::env::var_os("PAGEVISION_PRIVATE_EVAL")
        .map(PathBuf::from)
        .expect("set PAGEVISION_PRIVATE_EVAL to the private-eval folder")
}

fn read_json(path: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
#[ignore = "needs PAGEVISION_PRIVATE_EVAL"]
fn ink_matches_bakeoff_on_private_pages() {
    let eval = private_eval();
    // Written by the local script parity/ink_counts.py from bakeoff-scripts/common.py.
    let counts = read_json(&eval.join("parity/ink_counts.json"));
    let counts = counts.as_object().expect("ink_counts.json is an object");
    assert!(!counts.is_empty(), "ink_counts.json lists no page");
    let mut failures = Vec::new();
    for (n, (page, want)) in counts.iter().enumerate() {
        let png = std::fs::read(eval.join("pages").join(format!("{page}.png"))).unwrap();
        let ink = ink_mask(&flatten(&png).unwrap());
        let (_, comps) = components(&ink, 3);
        let order: u64 = comps
            .iter()
            .map(|c| u64::from(c.label) * (u64::from(c.x0) + 7 * u64::from(c.y0)))
            .sum();
        let got = (ink.count() as u64, comps.len() as u64, order);
        let want = (
            want["ink"].as_u64().unwrap(),
            want["components"].as_u64().unwrap(),
            want["order"].as_u64().unwrap(),
        );
        if got != want {
            // The page index only, so no private name lands in a log.
            failures.push(format!("page {n}: got {got:?}, want {want:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The public pages with region fixtures.
const PUBLIC_PAGES: [&str; 5] = ["test1", "test2", "diag1", "diag2", "diag3"];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn public_ink(page: &str) -> Mask {
    let path = repo_root().join(format!("tests/corpus/vision/{page}.png"));
    let png = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    ink_mask(&flatten(&png).unwrap())
}

fn rects(v: &serde_json::Value) -> Vec<Rect> {
    v.as_array()
        .expect("a list of boxes")
        .iter()
        .map(|b| {
            let c: Vec<f64> = b
                .as_array()
                .expect("a box")
                .iter()
                .map(|c| c.as_f64().unwrap())
                .collect();
            [c[0], c[1], c[2], c[3]]
        })
        .collect()
}

/// (drawn, bbox) pairs, sorted, for a multiset comparison.
fn region_set(regions: impl IntoIterator<Item = (bool, [i64; 4])>) -> Vec<(bool, [i64; 4])> {
    let mut v: Vec<_> = regions.into_iter().collect();
    v.sort();
    v
}

fn combine_none(ink: &Mask, v3: &[Rect], lines: &[Rect]) -> Vec<(bool, [i64; 4])> {
    region_set(
        combine_c(ink, v3, lines, RuleFixes::NONE)
            .into_iter()
            .map(|r| {
                // A possible drawing is a rule drawing box made half-open; the bake-off box
                // ends at its last pixel.
                let [x0, y0, x1, y1] = r.bbox;
                let bbox = if r.possible_drawing {
                    [x0, y0, x1 - 1, y1 - 1]
                } else {
                    r.bbox
                };
                (r.drawn, bbox)
            }),
    )
}

#[test]
fn combine_c_matches_bakeoff_on_public_pages() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/regions");
    for page in PUBLIC_PAGES {
        let lines = rects(&read_json(&dir.join(format!("{page}.lines.json"))));
        let v3 = rects(&read_json(&dir.join(format!("{page}.v3.json"))));
        let want = region_set(
            read_json(&dir.join(format!("{page}.c.json")))
                .as_array()
                .unwrap()
                .iter()
                .map(|r| {
                    let b = rects(&serde_json::json!([r[1]]))[0];
                    (r[0].as_bool().unwrap(), b.map(|c| c as i64))
                }),
        );
        assert_eq!(combine_none(&public_ink(page), &v3, &lines), want, "{page}");
    }
}

#[test]
#[ignore = "needs PAGEVISION_PRIVATE_EVAL"]
fn combine_c_matches_bakeoff_on_private_pages() {
    let eval = private_eval();
    let rb = eval.join("results/rb");
    let layout = read_json(&rb.join("PP-DocLayoutV3/regions.json"));
    let det = read_json(&rb.join("PP-OCRv6_small_det/regions.json"));
    let expected = read_json(&rb.join("C-V3+ocrv6s/regions.json"));
    let expected = expected.as_object().expect("regions.json is an object");
    assert!(!expected.is_empty(), "C-V3+ocrv6s lists no page");
    let boxes_of = |regions: &serde_json::Value, diagram: bool| -> Vec<Rect> {
        let picked: Vec<serde_json::Value> = regions
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| (r[0] == "diagram") == diagram)
            .map(|r| r[1].clone())
            .collect();
        rects(&serde_json::Value::Array(picked))
    };
    let mut failures = Vec::new();
    for (n, (page, want)) in expected.iter().enumerate() {
        let png = std::fs::read(eval.join("pages").join(format!("{page}.png"))).unwrap();
        let ink = ink_mask(&flatten(&png).unwrap());
        let v3 = boxes_of(&layout[page]["regions"], true);
        // hybrid.py: the detector's lines, or its text regions when it has no lines.
        let lines = match det[page]["lines"].as_array() {
            Some(l) if !l.is_empty() => rects(&det[page]["lines"]),
            _ => boxes_of(&det[page]["regions"], false),
        };
        let want = region_set(want["regions"].as_array().unwrap().iter().map(|r| {
            let b = rects(&serde_json::json!([r[1]]))[0];
            (r[0] == "diagram", b.map(|c| c as i64))
        }));
        let got = combine_none(&ink, &v3, &lines);
        if got != want {
            // The page index only, so no private name lands in a log.
            failures.push(format!("page {n}: got {got:?}, want {want:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn drawing_count(ink: &Mask, lines: &[Rect], fixes: RuleFixes) -> usize {
    ink_rules(ink, lines, fixes)
        .iter()
        .filter(|(k, _)| *k == RuleKind::Drawing)
        .count()
}

/// A no-regression check, not proof that the arrow fix works: with the stored PP-OCRv6 lines
/// the rules already see diag1 as one drawing without the fixes.
#[test]
fn diag1_stays_one_drawing_with_fixes() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/regions");
    let lines = rects(&read_json(&dir.join("diag1.lines.json")));
    let ink = public_ink("diag1");
    assert_eq!(drawing_count(&ink, &lines, RuleFixes::NONE), 1);
    assert_eq!(drawing_count(&ink, &lines, RuleFixes::ALL), 1);
}

#[test]
#[ignore = "needs PAGEVISION_PRIVATE_EVAL"]
fn private_underline_case_is_text_with_fixes() {
    let eval = private_eval();
    // A local file names the page with a real underline: {"underline": "<page id>"}.
    let cases = read_json(&eval.join("parity/rule_cases.json"));
    let page = cases["underline"]
        .as_str()
        .expect("parity/rule_cases.json names the underline page");
    let det = read_json(&eval.join("results/rb/PP-OCRv6_small_det/regions.json"));
    let lines = rects(&det[page]["lines"]);
    let png = std::fs::read(eval.join("pages").join(format!("{page}.png"))).unwrap();
    let ink = ink_mask(&flatten(&png).unwrap());
    assert!(drawing_count(&ink, &lines, RuleFixes::NONE) > 0);
    assert_eq!(drawing_count(&ink, &lines, RuleFixes::ALL), 0);
}

#[test]
fn snap_keeps_the_public_pages_readable() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/regions");
    for page in PUBLIC_PAGES {
        let lines = rects(&read_json(&dir.join(format!("{page}.lines.json"))));
        let v3 = rects(&read_json(&dir.join(format!("{page}.v3.json"))));
        let ink = public_ink(page);
        let proposed = combine_c(&ink, &v3, &lines, RuleFixes::ALL);
        let (snapped, notes) = snap(&ink, &proposed);
        assert!(notes.is_empty(), "{page}: {notes:?}");
        assert!(!snapped.is_empty(), "{page}");
        // Every drawn region survives: the public pages have ink in each.
        assert_eq!(
            snapped.iter().filter(|s| s.drawn).count(),
            v3.len(),
            "{page}"
        );
        for s in &snapped {
            let [x0, y0, x1, y1] = s.bbox;
            assert!(
                0 <= x0 && x0 < x1 && x1 <= i64::from(ink.width),
                "{page} {s:?}"
            );
            assert!(
                0 <= y0 && y0 < y1 && y1 <= i64::from(ink.height),
                "{page} {s:?}"
            );
            assert!(s.coverage > 0.0 && s.coverage <= 1.0, "{page} {s:?}");
        }
    }
}
