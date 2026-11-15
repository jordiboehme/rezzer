// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The scorecard: per-engine scores, an aggregate for the private set, and the fingerprint check.
//!
//! Private pages appear only as one aggregate, never by id.

use pagevision::model::{PREPROCESS_VERSION, QWEN35_4B};
use serde::{Deserialize, Serialize};

use crate::{
    BoxScore, EdgeScore, EvalError, Fingerprint, Manifest, PageEntry, PageRun, ResultsFile,
    line_cer, mermaid_edges, parse_truth, parse_truth_with_nodes, score_boxes, score_edges,
};

pub const FORMAT: &str = "pagevision-scorecard/1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageScore {
    pub id: String,
    pub cer: Option<f64>,
    pub edges: Option<EdgeScore>,
    pub boxes: Option<BoxScore>,
    pub known_fail: bool,
    pub truncated: bool,
    pub prompt_tokens: u32,
    pub output_tokens: u32,
    /// encode_ms + decode_ms.
    pub ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Aggregate {
    pub pages: u32,
    /// Mean over the pages with text truth; 0.0 when there are none.
    pub cer_mean: f64,
    /// Summed over the pages.
    pub edges: EdgeScore,
    pub ink_coverage_mean: f64,
    pub empty_boxes: u32,
    pub split_diagrams: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineScore {
    pub engine: String,
    pub baseline: bool,
    pub fingerprint: Fingerprint,
    pub public: Vec<PageScore>,
    pub private: Option<Aggregate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scorecard {
    /// "pagevision-scorecard/1".
    pub format: String,
    pub engines: Vec<EngineScore>,
}

impl Scorecard {
    pub fn new() -> Self {
        Self {
            format: FORMAT.to_string(),
            engines: Vec::new(),
        }
    }

    /// Adds the engine's entry, or replaces the one of the same name.
    pub fn upsert(&mut self, entry: EngineScore) {
        match self.engines.iter_mut().find(|e| e.engine == entry.engine) {
            Some(slot) => *slot = entry,
            None => self.engines.push(entry),
        }
    }
}

impl Default for Scorecard {
    fn default() -> Self {
        Self::new()
    }
}

fn read(path: &std::path::Path) -> Result<String, EvalError> {
    std::fs::read_to_string(path).map_err(|source| EvalError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn score_page(entry: &PageEntry, run: &PageRun) -> Result<PageScore, EvalError> {
    let cer = match &entry.text {
        Some(path) => Some(line_cer(&run.markdown, &read(path)?)),
        None => None,
    };
    let edges = match &entry.edges {
        Some(path) => {
            let raw = read(path)?;
            let truth = match &entry.nodes {
                Some(nodes) => parse_truth_with_nodes(&raw, &read(nodes)?),
                None => parse_truth(&raw),
            };
            Some(score_edges(&mermaid_edges(&run.markdown), &truth))
        }
        None => None,
    };
    let png = std::fs::read(&entry.image).map_err(|source| EvalError::Io {
        path: entry.image.clone(),
        source,
    })?;
    let boxes = Some(score_boxes(&png, &run.blocks, entry.diagrams)?);
    Ok(PageScore {
        id: entry.id.clone(),
        cer,
        edges,
        boxes,
        known_fail: entry.known_fail,
        truncated: run.truncated,
        prompt_tokens: run.prompt_tokens,
        output_tokens: run.output_tokens,
        ms: run.encode_ms + run.decode_ms,
    })
}

fn aggregate(pages: &[PageScore]) -> Aggregate {
    let cers: Vec<f64> = pages.iter().filter_map(|p| p.cer).collect();
    let mean = |v: &[f64]| {
        if v.is_empty() {
            0.0
        } else {
            v.iter().sum::<f64>() / v.len() as f64
        }
    };
    let coverage: Vec<f64> = pages
        .iter()
        .filter_map(|p| p.boxes.map(|b| b.ink_coverage))
        .collect();
    let mut edges = EdgeScore {
        truth: 0,
        predicted: 0,
        correct: 0,
    };
    let (mut empty_boxes, mut split_diagrams) = (0, 0);
    for p in pages {
        if let Some(e) = p.edges {
            edges.truth += e.truth;
            edges.predicted += e.predicted;
            edges.correct += e.correct;
        }
        if let Some(b) = p.boxes {
            empty_boxes += b.empty_boxes;
            split_diagrams += b.split_diagrams;
        }
    }
    Aggregate {
        pages: pages.len() as u32,
        cer_mean: mean(&cers),
        edges,
        ink_coverage_mean: mean(&coverage),
        empty_boxes,
        split_diagrams,
    }
}

/// Scores one results file against the manifests. Public pages are kept by id; pages of a
/// "private" manifest are folded into one aggregate. A page missing from the results is an error
/// (which does not name a private page).
pub fn score(
    manifests: &[Manifest],
    results: &ResultsFile,
    baseline: bool,
) -> Result<EngineScore, EvalError> {
    let mut public = Vec::new();
    let mut private = Vec::new();
    for manifest in manifests {
        let is_private = manifest.set == "private";
        for entry in &manifest.pages {
            let Some(run) = results.pages.iter().find(|r| r.id == entry.id) else {
                return Err(EvalError::Scoring(if is_private {
                    "a private page is missing from the results".to_string()
                } else {
                    format!("page {} is missing from the results", entry.id)
                }));
            };
            let scored = score_page(entry, run).map_err(|e| {
                if is_private {
                    EvalError::Scoring("a private page could not be scored".to_string())
                } else {
                    e
                }
            })?;
            if is_private {
                private.push(scored);
            } else {
                public.push(scored);
            }
        }
    }
    Ok(EngineScore {
        engine: results.engine.clone(),
        baseline,
        fingerprint: results.fingerprint.clone(),
        public,
        private: (!private.is_empty()).then(|| aggregate(&private)),
    })
}

/// The version of the `llama-cpp-2` package in a `Cargo.lock` text.
fn locked_llama_version(cargo_lock: &str) -> Option<String> {
    let mut lines = cargo_lock.lines();
    while let Some(line) = lines.next() {
        if line.trim() == "name = \"llama-cpp-2\"" {
            let version = lines.next()?.trim().strip_prefix("version = \"")?;
            return Some(version.strip_suffix('"')?.to_string());
        }
    }
    None
}

/// Checks every non-baseline engine against the build: the crate version (the workspace
/// version, as `pagevision::llama` writes it with `CARGO_PKG_VERSION`), the llama-cpp-2 version
/// in `cargo_lock`, the model SHA-256s of QWEN35_4B (model, then mmproj) and PREPROCESS_VERSION.
/// Baselines are skipped. Each error names the expected and the found value.
pub fn check_fingerprints(card: &Scorecard, cargo_lock: &str) -> Result<(), Vec<String>> {
    let locked = locked_llama_version(cargo_lock);
    let crate_version = env!("CARGO_PKG_VERSION");
    let preprocess = PREPROCESS_VERSION.to_string();
    let hashes = [QWEN35_4B.model.sha256, QWEN35_4B.mmproj.sha256];
    let mut errors = Vec::new();
    for engine in card.engines.iter().filter(|e| !e.baseline) {
        let (name, f) = (&engine.engine, &engine.fingerprint);
        if f.crate_version != crate_version {
            errors.push(format!(
                "{name}: crate version is {} but the build is {crate_version}",
                f.crate_version
            ));
        }
        match &locked {
            Some(v) if *v == f.llama_cpp_2 => {}
            Some(v) => errors.push(format!(
                "{name}: llama-cpp-2 is {} but Cargo.lock has {v}",
                f.llama_cpp_2
            )),
            None => errors.push(format!(
                "{name}: llama-cpp-2 is {} but Cargo.lock has no llama-cpp-2 entry",
                f.llama_cpp_2
            )),
        }
        if f.model_sha256 != hashes {
            errors.push(format!(
                "{name}: model SHA-256s are {:?} but the build expects {:?}",
                f.model_sha256, hashes
            ));
        }
        if f.preprocess != preprocess {
            errors.push(format!(
                "{name}: preprocess version is {} but the build is {preprocess}",
                f.preprocess
            ));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// A Markdown table: public pages by id, the private set as one aggregate row per engine.
pub fn render_markdown(engine: &EngineScore) -> String {
    let mut out = format!(
        "## {}{}\n\n| page | cer | edges | ink | empty | split | tokens | ms | flags |\n|---|---|---|---|---|---|---|---|---|\n",
        engine.engine,
        if engine.baseline { " (baseline)" } else { "" }
    );
    for p in &engine.public {
        let cer = p.cer.map_or("-".into(), |c| format!("{c:.3}"));
        let edges = p.edges.map_or("-".into(), |e| {
            format!("{}/{}/{}", e.correct, e.predicted, e.truth)
        });
        let (ink, empty, split) = p.boxes.map_or(("-".into(), "-".into(), "-".into()), |b| {
            (
                format!("{:.3}", b.ink_coverage),
                b.empty_boxes.to_string(),
                b.split_diagrams.to_string(),
            )
        });
        let mut flags = Vec::new();
        if p.known_fail {
            flags.push("known-fail");
        }
        if p.truncated {
            flags.push("truncated");
        }
        out.push_str(&format!(
            "| {} | {cer} | {edges} | {ink} | {empty} | {split} | {}/{} | {} | {} |\n",
            p.id,
            p.prompt_tokens,
            p.output_tokens,
            p.ms,
            flags.join(" ")
        ));
    }
    if let Some(a) = &engine.private {
        out.push_str(&format!(
            "| private set ({} pages) | {:.3} | {}/{}/{} | {:.3} | {} | {} | - | - | |\n",
            a.pages,
            a.cer_mean,
            a.edges.correct,
            a.edges.predicted,
            a.edges.truth,
            a.ink_coverage_mean,
            a.empty_boxes,
            a.split_diagrams
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RunBlock;
    use std::path::Path;

    const LOCK: &str = "[[package]]\nname = \"other\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"llama-cpp-2\"\nversion = \"0.1.158\"\n";

    fn fingerprint(llama: &str) -> Fingerprint {
        Fingerprint {
            crate_version: env!("CARGO_PKG_VERSION").to_string(),
            llama_cpp_2: llama.to_string(),
            model_sha256: vec![
                QWEN35_4B.model.sha256.to_string(),
                QWEN35_4B.mmproj.sha256.to_string(),
            ],
            prompt_sha256: "p".to_string(),
            preprocess: PREPROCESS_VERSION.to_string(),
        }
    }

    fn engine(name: &str, baseline: bool, f: Fingerprint) -> EngineScore {
        EngineScore {
            engine: name.to_string(),
            baseline,
            fingerprint: f,
            public: vec![],
            private: None,
        }
    }

    fn png(dir: &Path, name: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        let mut img = image::RgbImage::from_pixel(40, 40, image::Rgb([255, 255, 255]));
        for x in 5..35 {
            img.put_pixel(x, 20, image::Rgb([0, 0, 0]));
        }
        img.save(&path).unwrap();
        path
    }

    fn page(dir: &Path, id: &str, known_fail: bool) -> PageEntry {
        let text = dir.join(format!("{id}.txt"));
        std::fs::write(&text, "hello\n").unwrap();
        PageEntry {
            id: id.to_string(),
            image: png(dir, &format!("{id}.png")),
            text: Some(text),
            edges: None,
            nodes: None,
            diagrams: 0,
            known_fail,
        }
    }

    fn run(id: &str) -> PageRun {
        PageRun {
            id: id.to_string(),
            markdown: "hello".to_string(),
            truncated: false,
            prompt_tokens: 10,
            output_tokens: 2,
            encode_ms: 3,
            decode_ms: 4,
            blocks: vec![RunBlock {
                kind: "paragraph".to_string(),
                label: None,
                bbox: Some([0, 0, 1000, 1000]),
                markdown: "hello".to_string(),
            }],
        }
    }

    fn results(ids: &[&str]) -> ResultsFile {
        ResultsFile {
            format: crate::results::FORMAT.to_string(),
            engine: "e".to_string(),
            fingerprint: fingerprint("0.1.158"),
            pages: ids.iter().map(|i| run(i)).collect(),
        }
    }

    #[test]
    fn private_pages_only_aggregate() {
        let dir = tempfile::tempdir().unwrap();
        let public = Manifest {
            set: "public".into(),
            pages: vec![page(dir.path(), "pub1", false)],
        };
        let private = Manifest {
            set: "private".into(),
            pages: vec![
                page(dir.path(), "secret1", false),
                page(dir.path(), "secret2", false),
            ],
        };
        let s = score(
            &[public, private],
            &results(&["pub1", "secret1", "secret2"]),
            false,
        )
        .unwrap();
        assert_eq!(s.public.len(), 1);
        assert_eq!(s.public[0].id, "pub1");
        assert_eq!(s.public[0].ms, 7);
        let agg = s.private.clone().unwrap();
        assert_eq!(agg.pages, 2);
        assert_eq!(agg.cer_mean, 0.0);
        assert_eq!(agg.ink_coverage_mean, 1.0);
        let json = serde_json::to_string(&s).unwrap();
        let md = render_markdown(&s);
        assert!(!json.contains("secret") && !md.contains("secret"));
        assert!(md.contains("pub1") && md.contains("private set (2 pages)"));
    }

    #[test]
    fn missing_page_error_hides_private_id() {
        let dir = tempfile::tempdir().unwrap();
        let public = Manifest {
            set: "public".into(),
            pages: vec![page(dir.path(), "pub1", false)],
        };
        let err = score(&[public], &results(&[]), false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("pub1"));
        let private = Manifest {
            set: "private".into(),
            pages: vec![page(dir.path(), "secret1", false)],
        };
        let err = score(&[private], &results(&[]), false)
            .unwrap_err()
            .to_string();
        assert!(!err.contains("secret1"));
        assert!(err.contains("a private page is missing from the results"));
    }

    #[test]
    fn edges_use_node_map_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = page(dir.path(), "d", false);
        let edges = dir.path().join("d.edges");
        let nodes = dir.path().join("d.nodes");
        std::fs::write(&edges, "A --> B\n").unwrap();
        std::fs::write(&nodes, "A\talpha\nB\tbeta\n").unwrap();
        p.edges = Some(edges);
        p.nodes = Some(nodes);
        let mut r = run("d");
        r.markdown = "```mermaid\nflowchart LR\n a[alpha] --> b[beta]\n```".into();
        let m = Manifest {
            set: "public".into(),
            pages: vec![p],
        };
        let mut rf = results(&[]);
        rf.pages.push(r);
        let s = score(&[m], &rf, false).unwrap();
        let e = s.public[0].edges.unwrap();
        assert_eq!((e.truth, e.correct), (1, 1));
    }

    #[test]
    fn known_fail_kept_but_flagged() {
        let dir = tempfile::tempdir().unwrap();
        let m = Manifest {
            set: "public".into(),
            pages: vec![page(dir.path(), "bad", true)],
        };
        let s = score(&[m], &results(&["bad"]), true).unwrap();
        assert_eq!(s.public.len(), 1);
        assert!(s.public[0].known_fail);
        assert!(s.baseline);
        assert!(render_markdown(&s).contains("known-fail"));
    }

    #[test]
    fn baseline_fingerprint_not_checked() {
        let mut f = fingerprint("0.0.1");
        f.crate_version = "notedown-ocr 0.1.0".into();
        f.preprocess = "none".into();
        f.model_sha256 = vec!["x".into()];
        let card = Scorecard {
            format: FORMAT.into(),
            engines: vec![engine("chandra", true, f)],
        };
        assert!(check_fingerprints(&card, LOCK).is_ok());
    }

    #[test]
    fn stale_engine_fingerprint_fails() {
        let card = Scorecard {
            format: FORMAT.into(),
            engines: vec![engine("qwen", false, fingerprint("0.1.150"))],
        };
        let errs = check_fingerprints(&card, LOCK).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("0.1.150") && errs[0].contains("0.1.158"));
        let mut f = fingerprint("0.1.158");
        f.preprocess = "0".into();
        f.model_sha256 = vec!["abc".into()];
        let card = Scorecard {
            format: FORMAT.into(),
            engines: vec![engine("qwen", false, f)],
        };
        let errs = check_fingerprints(&card, LOCK).unwrap_err();
        assert_eq!(errs.len(), 2);
        assert!(errs.iter().any(|e| e.contains("abc")));
        assert!(errs.iter().any(|e| e.contains("preprocess")));
    }

    #[test]
    fn current_fingerprint_passes() {
        let card = Scorecard {
            format: FORMAT.into(),
            engines: vec![engine("qwen", false, fingerprint("0.1.158"))],
        };
        assert!(check_fingerprints(&card, LOCK).is_ok());
    }

    #[test]
    fn scorecard_matches_build() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let card_path = root.join("crates/formats/pagevision/eval/scorecard.json");
        let Ok(raw) = std::fs::read_to_string(&card_path) else {
            return; // not written yet
        };
        let card: Scorecard = serde_json::from_str(&raw).unwrap();
        let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
        if let Err(errors) = check_fingerprints(&card, &lock) {
            panic!("scorecard is stale:\n{}", errors.join("\n"));
        }
    }
}
