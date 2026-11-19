// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The model regressions, checked on a results file of a full run (`eval_run`), given by
//! `PAGEVISION_RESULTS`. The private check also needs `PAGEVISION_PRIVATE_EVAL`. A page the
//! checks need that is missing from the results fails the test.
//!
//! - A misread label is a truth node that no predicted label maps to (with the label mapping of
//!   the edge score).
//! - An invented node is a predicted label that maps to no truth node or arrow label, beyond the
//!   one each misread label explains. An invented arrow joins two truth nodes that the truth does
//!   not join. A reversed arrow is wrong, not invented.
//! - The known fail (diag3) is left out of the invented check, as the scorecard flags it.

use std::path::{Path, PathBuf};

use pagevision_eval::{
    Manifest, PageEntry, PageRun, ResultsFile, TruthEdges, check_nodes, load_manifest,
    load_results, mermaid_edges, parse_truth, parse_truth_with_nodes, score_edges,
};

fn env_path(name: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {name}"))
}

fn results() -> ResultsFile {
    load_results(&env_path("PAGEVISION_RESULTS")).expect("a results file")
}

fn public() -> Manifest {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/corpus/vision/manifest.toml");
    load_manifest(&path).expect("the public manifest")
}

fn entry<'a>(m: &'a Manifest, id: &str) -> &'a PageEntry {
    m.pages
        .iter()
        .find(|p| p.id == id)
        .unwrap_or_else(|| panic!("page {id} is missing from the public manifest"))
}

fn run<'a>(r: &'a ResultsFile, id: &str) -> &'a PageRun {
    r.pages
        .iter()
        .find(|p| p.id == id)
        .unwrap_or_else(|| panic!("page {id} is missing from the results"))
}

fn truth(e: &PageEntry) -> TruthEdges {
    let read = |p: &Path| {
        std::fs::read_to_string(p).unwrap_or_else(|err| panic!("{}: {err}", p.display()))
    };
    let edges = read(e.edges.as_ref().expect("an edges truth"));
    match &e.nodes {
        Some(nodes) => parse_truth_with_nodes(&edges, &read(nodes)),
        None => parse_truth(&edges),
    }
}

#[test]
#[ignore = "needs a results file of a full run"]
fn test2_edges_exact() {
    let (r, m) = (results(), public());
    let e = entry(&m, "test2");
    let edges = mermaid_edges(&run(&r, "test2").markdown);
    let score = score_edges(&edges, &truth(e));
    eprintln!("test2: {score:?} {edges:?}");
    assert_eq!(
        (score.correct, score.predicted, score.truth),
        (5, 5, 5),
        "{edges:?}"
    );
}

#[test]
#[ignore = "needs a results file of a full run"]
fn diag1_and_diag2_at_most_one_misread_label() {
    let (r, m) = (results(), public());
    for id in ["diag1", "diag2"] {
        let check = check_nodes(&mermaid_edges(&run(&r, id).markdown), &truth(entry(&m, id)));
        eprintln!("{id}: {check:?}");
        assert!(check.misread.len() <= 1, "{id}: {check:?}");
    }
}

#[test]
#[ignore = "needs a results file of a full run"]
fn no_invented_nodes_on_public_pages() {
    let (r, m) = (results(), public());
    let mut problems = Vec::new();
    for e in &m.pages {
        let edges = mermaid_edges(&run(&r, &e.id).markdown);
        if e.known_fail {
            eprintln!("{}: known fail, not checked", e.id);
            continue;
        }
        if e.edges.is_none() {
            if !edges.is_empty() {
                problems.push(format!(
                    "{}: edges on a page without a diagram: {edges:?}",
                    e.id
                ));
            }
            continue;
        }
        let check = check_nodes(&edges, &truth(e));
        eprintln!("{}: {check:?}", e.id);
        if check.invented_nodes() > 0 || !check.invented_arrows.is_empty() {
            problems.push(format!("{}: {check:?}", e.id));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

/// The private pages with sketches and no diagram give no Mermaid at all. The sketch block
/// count is printed, not checked.
#[test]
#[ignore = "needs a results file of a full run and the private set"]
fn sketch_pages_have_no_mermaid() {
    let r = results();
    let m = load_manifest(&env_path("PAGEVISION_PRIVATE_EVAL").join("manifest.toml"))
        .expect("the private manifest");
    let pages: Vec<&PageEntry> = m
        .pages
        .iter()
        .filter(|p| p.sketches > 0 && p.diagrams == 0)
        .collect();
    assert!(
        !pages.is_empty(),
        "the private manifest has no page with sketches and no diagram"
    );
    for (i, e) in pages.iter().enumerate() {
        let page = r
            .pages
            .iter()
            .find(|p| p.id == e.id)
            .unwrap_or_else(|| panic!("sketch page {i} is missing from the results"));
        let sketches = page.markdown.matches("> Sketch:").count();
        eprintln!(
            "sketch page {i}: {sketches} sketch blocks ({} drawn)",
            e.sketches
        );
        assert!(
            !page.markdown.contains("```mermaid"),
            "sketch page {i} has a Mermaid block"
        );
    }
}
