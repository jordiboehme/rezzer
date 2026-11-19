// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `regions_from` on the public pages over the committed detector fixtures
//! (`tests/fixtures/detect/`): the snapped output, without the models.

use std::path::{Path, PathBuf};

use pagevision::ink::flatten;
use pagevision::layoutpost::DRAWN_CLASSES;
use pagevision::{RegionKind, Regions, regions_from};

fn read_json(path: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn boxes<'a>(v: impl IntoIterator<Item = &'a serde_json::Value>) -> Vec<[f64; 4]> {
    v.into_iter()
        .map(|b| {
            let c: Vec<f64> = b
                .as_array()
                .expect("a box")
                .iter()
                .map(|n| n.as_f64().expect("a number"))
                .collect();
            [c[0], c[1], c[2], c[3]]
        })
        .collect()
}

fn public_regions(page: &str) -> Regions {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixtures = root.join("tests/fixtures/detect");
    let v3 = read_json(&fixtures.join(format!("{page}.v3.json")));
    let drawn = boxes(
        v3["final"]
            .as_array()
            .expect("a list")
            .iter()
            .filter(|r| DRAWN_CLASSES.contains(&(r[0].as_u64().expect("a class") as usize)))
            .map(|r| &r[2]),
    );
    let v6 = read_json(&fixtures.join(format!("{page}.v6.json")));
    let lines = boxes(v6["lines"].as_array().expect("a list"));
    let png = std::fs::read(root.join(format!("../../../tests/corpus/vision/{page}.png"))).unwrap();
    regions_from(&flatten(&png).unwrap(), &drawn, &lines)
}

/// Per page: drawings, text regions, possible missed drawings. The drawings are the drawn
/// regions of the fixtures (6 on the five pages); the text counts were measured with this code.
/// The diagram labels belong to the drawings, so the diagram pages have no text region.
#[test]
fn regions_of_public_fixtures() {
    let want = [
        // test1 is one text block (heading, paragraph and list): C gives coarse blocks.
        ("test1", 0, 1, 0),
        ("test2", 2, 1, 0),
        ("diag1", 1, 0, 0),
        ("diag2", 1, 0, 0),
        ("diag3", 2, 0, 0),
    ];
    let mut got = Vec::new();
    for (page, ..) in want {
        let r = public_regions(page);
        assert!(r.notes.is_empty(), "{page}: {:?}", r.notes);
        let count = |k: RegionKind| r.regions.iter().filter(|x| x.kind == k).count();
        let notes = r.regions.iter().filter(|x| x.note.is_some()).count();
        got.push((
            page,
            count(RegionKind::Drawing),
            count(RegionKind::Text),
            notes,
        ));
        // Reading order and the grid.
        for pair in r.regions.windows(2) {
            let (a, b) = (pair[0].bbox, pair[1].bbox);
            assert!((a.y0, a.x0) <= (b.y0, b.x0), "{page}: {a:?} before {b:?}");
        }
        for x in &r.regions {
            let b = x.bbox;
            assert!(b.x0 < b.x1 && b.x1 <= 1000 && b.y0 < b.y1 && b.y1 <= 1000);
            assert!(x.certainty > 0.0 && x.certainty <= 1.0, "{page}: {x:?}");
        }
    }
    assert_eq!(got, want);
}
