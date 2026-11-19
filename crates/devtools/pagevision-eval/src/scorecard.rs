// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The scorecard: per-engine scores, an aggregate for the private set, and the fingerprint check.
//!
//! Private pages appear only as one aggregate, never by id.

use pagevision::ink::INK_VERSION;
use pagevision::model::{LAYOUT_V3, PREPROCESS_VERSION, QWEN35_4B, TEXT_DET_V6};
use pagevision::rules::RULES_VERSION;
use serde::{Deserialize, Serialize};

use crate::{
    BoxScore, EdgeScore, EvalError, Fingerprint, Manifest, PageEntry, PageRun, RegionScore,
    ResultsFile, line_cer, mermaid_edges, parse_truth, parse_truth_with_nodes, score_boxes,
    score_edges, score_regions,
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
    /// The region score, for a page with region truth.
    #[serde(default)]
    pub regions: Option<RegionScore>,
    /// The number of regions of a regions-only run, otherwise of blocks.
    #[serde(default)]
    pub blocks: u32,
}

/// The region scores of a set of pages, summed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegionSums {
    /// Pages with region truth.
    pub pages: u32,
    /// Of them, the pages that pass the page rule.
    pub ok: u32,
    pub truth: u32,
    pub hit: u32,
    pub split: u32,
    pub false_drawings: u32,
    pub text_cov_mean: f64,
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
    /// The region scores, when any page has region truth.
    #[serde(default)]
    pub regions: Option<RegionSums>,
    /// Pages with a text or box measurement; 0 for a regions-only run, whose text and box
    /// means are then not measured. Absent in older scorecards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_pages: Option<u32>,
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

/// A region truth file: the diagram boxes in page pixels.
#[derive(Deserialize)]
struct RegionTruth {
    diagrams: Vec<[i64; 4]>,
}

/// A run that only proposed regions: it read no text, so the text, edge and box metrics are not
/// measured.
fn regions_only_run(run: &PageRun) -> bool {
    !run.regions.is_empty() && run.blocks.is_empty() && run.markdown.is_empty()
}

fn score_page(entry: &PageEntry, run: &PageRun) -> Result<PageScore, EvalError> {
    let read_run = !regions_only_run(run);
    let cer = match &entry.text {
        Some(path) if read_run => Some(line_cer(&run.markdown, &read(path)?)),
        _ => None,
    };
    let edges = match &entry.edges {
        Some(path) if read_run => {
            let raw = read(path)?;
            let truth = match &entry.nodes {
                Some(nodes) => parse_truth_with_nodes(&raw, &read(nodes)?),
                None => parse_truth(&raw),
            };
            Some(score_edges(&mermaid_edges(&run.markdown), &truth))
        }
        _ => None,
    };
    let png = std::fs::read(&entry.image).map_err(|source| EvalError::Io {
        path: entry.image.clone(),
        source,
    })?;
    let boxes = if read_run {
        Some(score_boxes(&png, &run.blocks, entry.diagrams)?)
    } else {
        None
    };
    let regions = match &entry.regions {
        Some(path) if !run.regions.is_empty() => {
            let truth: RegionTruth = serde_json::from_str(&read(path)?)?;
            Some(score_regions(&png, &truth.diagrams, &run.regions)?)
        }
        _ => None,
    };
    let blocks = if run.regions.is_empty() {
        run.blocks.len()
    } else {
        run.regions.len()
    };
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
        regions,
        blocks: blocks as u32,
    })
}

fn region_sums(pages: &[PageScore]) -> Option<RegionSums> {
    let scores: Vec<RegionScore> = pages.iter().filter_map(|p| p.regions).collect();
    if scores.is_empty() {
        return None;
    }
    let sum = |f: fn(&RegionScore) -> u32| scores.iter().map(f).sum::<u32>();
    Some(RegionSums {
        pages: scores.len() as u32,
        ok: scores.iter().filter(|s| s.ok).count() as u32,
        truth: sum(|s| s.truth),
        hit: sum(|s| s.hit),
        split: sum(|s| s.split),
        false_drawings: sum(|s| s.false_drawings),
        text_cov_mean: scores.iter().map(|s| s.text_cov).sum::<f64>() / scores.len() as f64,
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
        regions: region_sums(pages),
        read_pages: Some(
            pages
                .iter()
                .filter(|p| p.cer.is_some() || p.boxes.is_some())
                .count() as u32,
        ),
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
    if baseline && let Some(problem) = baseline_problem(&results.fingerprint) {
        return Err(EvalError::Scoring(format!(
            "{} cannot be a baseline: it {problem}",
            results.engine
        )));
    }
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

/// The version of the package `name` in a `Cargo.lock` text.
fn locked_version(cargo_lock: &str, name: &str) -> Option<String> {
    let header = format!("name = \"{name}\"");
    let mut lines = cargo_lock.lines();
    while let Some(line) = lines.next() {
        if line.trim() == header {
            let version = lines.next()?.trim().strip_prefix("version = \"")?;
            return Some(version.strip_suffix('"')?.to_string());
        }
    }
    None
}

/// A regions-only engine ran the detectors and no vision model: no model hashes, a layout hash.
fn regions_only(f: &Fingerprint) -> bool {
    f.model_sha256.is_empty() && !f.layout_sha256.is_empty()
}

/// Checks every non-baseline engine against the build.
///
/// - Every engine: the candle version in `cargo_lock` (candle-core and candle-nn), the detector
///   SHA-256s of LAYOUT_V3 and TEXT_DET_V6, RULES_VERSION and INK_VERSION.
/// - Every engine but a regions-only one (no model hashes, see `regions_only`): also the crate
///   version (the workspace version, as `pagevision::llama` writes it with `CARGO_PKG_VERSION`),
///   the llama-cpp-2 version in `cargo_lock`, the model SHA-256s of QWEN35_4B (model, then
///   mmproj), the SHA-256 of the build's prompts (`prompts::joined`) and PREPROCESS_VERSION.
///
/// Baselines are skipped, except that one using the QWEN35_4B hashes or a preprocess other than
/// "none" is an error. Each error names the expected and the found value.
pub fn check_fingerprints(card: &Scorecard, cargo_lock: &str) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    for engine in &card.engines {
        let (name, f) = (&engine.engine, &engine.fingerprint);
        if engine.baseline {
            if let Some(problem) = baseline_problem(f) {
                errors.push(format!("{name}: a baseline entry {problem}"));
            }
            continue;
        }
        check_detectors(name, f, cargo_lock, &mut errors);
        if !regions_only(f) {
            check_model(name, f, cargo_lock, &mut errors);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// The detector part of the fingerprint: candle, the detector files, the rules and the ink.
fn check_detectors(name: &str, f: &Fingerprint, cargo_lock: &str, errors: &mut Vec<String>) {
    for package in ["candle-core", "candle-nn"] {
        match locked_version(cargo_lock, package) {
            Some(v) if v == f.candle => {}
            Some(v) => errors.push(format!(
                "{name}: candle is {:?} but Cargo.lock has {package} {v}",
                f.candle
            )),
            None => errors.push(format!(
                "{name}: candle is {:?} but Cargo.lock has no {package} entry",
                f.candle
            )),
        }
    }
    if f.layout_sha256 != LAYOUT_V3.sha256 {
        errors.push(format!(
            "{name}: layout detector SHA-256 is {:?} but the build expects {}",
            f.layout_sha256, LAYOUT_V3.sha256
        ));
    }
    if f.text_det_sha256 != TEXT_DET_V6.sha256 {
        errors.push(format!(
            "{name}: text line detector SHA-256 is {:?} but the build expects {}",
            f.text_det_sha256, TEXT_DET_V6.sha256
        ));
    }
    if f.rules != RULES_VERSION {
        errors.push(format!(
            "{name}: rules version is {} but the build is {RULES_VERSION}",
            f.rules
        ));
    }
    if f.ink != INK_VERSION {
        errors.push(format!(
            "{name}: ink version is {} but the build is {INK_VERSION}",
            f.ink
        ));
    }
}

/// The vision model part of the fingerprint: the crate, llama-cpp-2, the model files and the
/// preprocessing.
fn check_model(name: &str, f: &Fingerprint, cargo_lock: &str, errors: &mut Vec<String>) {
    let locked = locked_version(cargo_lock, "llama-cpp-2");
    let locked_sys = locked_version(cargo_lock, "llama-cpp-sys-2");
    let crate_version = env!("CARGO_PKG_VERSION");
    let preprocess = PREPROCESS_VERSION.to_string();
    let hashes = [QWEN35_4B.model.sha256, QWEN35_4B.mmproj.sha256];
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
    // llama-cpp-sys-2 holds the native llama.cpp; it must be locked to the same version.
    match &locked_sys {
        Some(v) if *v == f.llama_cpp_2 => {}
        Some(v) => errors.push(format!(
            "{name}: llama-cpp-2 is {} but Cargo.lock has llama-cpp-sys-2 {v}",
            f.llama_cpp_2
        )),
        None => errors.push(format!(
            "{name}: llama-cpp-2 is {} but Cargo.lock has no llama-cpp-sys-2 entry",
            f.llama_cpp_2
        )),
    }
    if f.model_sha256 != hashes {
        errors.push(format!(
            "{name}: model SHA-256s are {:?} but the build expects {:?}",
            f.model_sha256, hashes
        ));
    }
    let prompts = pagevision::model::prompt_sha256(&pagevision::prompts::joined());
    if f.prompt_sha256 != prompts {
        errors.push(format!(
            "{name}: prompt SHA-256 is {} but the build's prompts hash to {prompts}",
            f.prompt_sha256
        ));
    }
    if f.preprocess != preprocess {
        errors.push(format!(
            "{name}: preprocess version is {} but the build is {preprocess}",
            f.preprocess
        ));
    }
}

/// Why an entry cannot be a baseline: a baseline is skipped by `check_fingerprints`, so it must
/// not be a pagevision run (the Qwen model hashes, or a numeric preprocess version).
fn baseline_problem(f: &Fingerprint) -> Option<String> {
    if f.model_sha256 == [QWEN35_4B.model.sha256, QWEN35_4B.mmproj.sha256] {
        Some("uses the QWEN35_4B model hashes".to_string())
    } else if f.preprocess != "none" {
        Some(format!("has preprocess {:?}, not \"none\"", f.preprocess))
    } else {
        None
    }
}

/// A Markdown table: public pages by id, the private set as one aggregate row per engine.
pub fn render_markdown(engine: &EngineScore) -> String {
    let mut out = format!(
        "## {}{}\n\n| page | cer | edges | ink | empty | split | regions | blocks | tokens | ms | flags |\n|---|---|---|---|---|---|---|---|---|---|---|\n",
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
        let regions = p.regions.map_or("-".into(), |r| {
            format!(
                "{} hit {}/{} split {} false {} text {:.3}",
                if r.ok { "ok" } else { "FAIL" },
                r.hit,
                r.truth,
                r.split,
                r.false_drawings,
                r.text_cov
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
            "| {} | {cer} | {edges} | {ink} | {empty} | {split} | {regions} | {} | {}/{} | {} | {} |\n",
            p.id,
            p.blocks,
            p.prompt_tokens,
            p.output_tokens,
            p.ms,
            flags.join(" ")
        ));
    }
    if let Some(a) = &engine.private {
        let regions = a.regions.as_ref().map_or("-".into(), |r| {
            format!(
                "ok {}/{} hit {}/{} split {} false {} text {:.3}",
                r.ok, r.pages, r.hit, r.truth, r.split, r.false_drawings, r.text_cov_mean
            )
        });
        let (cer, edges, ink, empty, split) = if a.read_pages == Some(0) {
            ("-".into(), "-".into(), "-".into(), "-".into(), "-".into())
        } else {
            (
                format!("{:.3}", a.cer_mean),
                format!(
                    "{}/{}/{}",
                    a.edges.correct, a.edges.predicted, a.edges.truth
                ),
                format!("{:.3}", a.ink_coverage_mean),
                a.empty_boxes.to_string(),
                a.split_diagrams.to_string(),
            )
        };
        out.push_str(&format!(
            "| private set ({} pages) | {cer} | {edges} | {ink} | {empty} | {split} | {regions} | - | - | - | |\n",
            a.pages,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RunBlock;
    use std::path::Path;

    const LOCK: &str = "[[package]]\nname = \"candle-core\"\nversion = \"0.11.1\"\n\n[[package]]\nname = \"candle-nn\"\nversion = \"0.11.1\"\n\n[[package]]\nname = \"other\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"llama-cpp-2\"\nversion = \"0.1.158\"\n\n[[package]]\nname = \"llama-cpp-sys-2\"\nversion = \"0.1.158\"\n";

    fn fingerprint(llama: &str) -> Fingerprint {
        Fingerprint {
            crate_version: env!("CARGO_PKG_VERSION").to_string(),
            llama_cpp_2: llama.to_string(),
            model_sha256: vec![
                QWEN35_4B.model.sha256.to_string(),
                QWEN35_4B.mmproj.sha256.to_string(),
            ],
            prompt_sha256: pagevision::model::prompt_sha256(&pagevision::prompts::joined()),
            preprocess: PREPROCESS_VERSION.to_string(),
            ..regions_fingerprint()
        }
    }

    /// A regions-only engine as `eval_run --regions-only` writes it.
    fn regions_fingerprint() -> Fingerprint {
        Fingerprint::from(&pagevision::regions::regions_fingerprint())
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
        // A short stroke: a run of half the page width or more would be a ruled line, not ink.
        let mut img = image::RgbImage::from_pixel(40, 40, image::Rgb([255, 255, 255]));
        for x in 5..15 {
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
            regions: None,
            sketches: 0,
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
            regions: Vec::new(),
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
        let mut r = results(&["bad"]);
        r.fingerprint.model_sha256 = vec!["x".into()];
        r.fingerprint.preprocess = "none".into();
        let s = score(&[m], &r, true).unwrap();
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
        // llama-cpp-2 and llama-cpp-sys-2 both differ from the fingerprint.
        assert_eq!(errs.len(), 2);
        assert!(
            errs.iter()
                .all(|e| e.contains("0.1.150") && e.contains("0.1.158"))
        );
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
    fn drifted_sys_crate_fails() {
        let lock = LOCK.replace(
            "name = \"llama-cpp-sys-2\"\nversion = \"0.1.158\"",
            "name = \"llama-cpp-sys-2\"\nversion = \"0.1.161\"",
        );
        let card = Scorecard {
            format: FORMAT.into(),
            engines: vec![engine("qwen", false, fingerprint("0.1.158"))],
        };
        let errs = check_fingerprints(&card, &lock).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("llama-cpp-sys-2") && errs[0].contains("0.1.161"));
        let no_sys = LOCK
            .split("[[package]]\nname = \"llama-cpp-sys-2\"")
            .next()
            .unwrap();
        assert!(check_fingerprints(&card, no_sys).is_err());
    }

    #[test]
    fn qwen_run_cannot_be_a_baseline() {
        let card = Scorecard {
            format: FORMAT.into(),
            engines: vec![engine("qwen", true, fingerprint("0.1.158"))],
        };
        assert!(check_fingerprints(&card, LOCK).is_err());
        let mut f = fingerprint("0.1.158");
        f.model_sha256 = vec!["x".into()];
        let card = Scorecard {
            format: FORMAT.into(),
            engines: vec![engine("chandra", true, f.clone())],
        };
        assert!(check_fingerprints(&card, LOCK).is_err());
        f.preprocess = "none".into();
        let mut r = results(&[]);
        r.fingerprint = f;
        assert!(score(&[], &r, true).is_ok());
        r.fingerprint = fingerprint("0.1.158");
        assert!(score(&[], &r, true).is_err());
        assert!(score(&[], &r, false).is_ok());
    }

    #[test]
    fn current_fingerprint_passes() {
        let card = Scorecard {
            format: FORMAT.into(),
            engines: vec![engine("qwen", false, fingerprint("0.1.158"))],
        };
        assert!(check_fingerprints(&card, LOCK).is_ok());
    }

    /// A changed prompt needs a new run: the prompt hash is checked against the build's prompts.
    #[test]
    fn fingerprint_check_names_a_changed_prompt() {
        let mut f = fingerprint("0.1.158");
        f.prompt_sha256 = "0".repeat(64);
        let card = Scorecard {
            format: FORMAT.into(),
            engines: vec![engine("qwen", false, f)],
        };
        let errs = check_fingerprints(&card, LOCK).unwrap_err();
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].contains("prompt"), "{errs:?}");
    }

    #[test]
    fn fingerprint_check_names_a_changed_rules_version() {
        let mut f = regions_fingerprint();
        f.rules += 1;
        let card = Scorecard {
            format: FORMAT.into(),
            engines: vec![engine("pagevision-c-regions", false, f)],
        };
        let errs = check_fingerprints(&card, LOCK).unwrap_err();
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].contains("rules version"), "{errs:?}");
        assert!(errs[0].contains(&RULES_VERSION.to_string()));
    }

    #[test]
    fn regions_only_engine_skips_the_model_checks() {
        // No llama-cpp-2 in the lock, no model hashes, another preprocess and crate version.
        let lock = "[[package]]\nname = \"candle-core\"\nversion = \"0.11.1\"\n\n[[package]]\nname = \"candle-nn\"\nversion = \"0.11.1\"\n";
        let mut f = regions_fingerprint();
        f.preprocess = "0".into();
        f.crate_version = "0.0.1".into();
        let card = Scorecard {
            format: FORMAT.into(),
            engines: vec![engine("pagevision-c-regions", false, f.clone())],
        };
        assert_eq!(check_fingerprints(&card, lock), Ok(()));
        // Each detector field is checked.
        let changes: [fn(&mut Fingerprint); 5] = [
            |f| f.candle = "0.11.0".into(),
            |f| f.layout_sha256 = "x".into(),
            |f| f.text_det_sha256 = "y".into(),
            |f| f.ink += 1,
            |f| f.rules += 1,
        ];
        for change in changes {
            let mut g = f.clone();
            change(&mut g);
            let card = Scorecard {
                format: FORMAT.into(),
                engines: vec![engine("r", false, g)],
            };
            assert!(check_fingerprints(&card, lock).is_err());
        }
        // A candle bump in the lock fails, naming both packages.
        let bumped = lock.replace("0.11.1", "0.11.2");
        let errs = check_fingerprints(&card, &bumped).unwrap_err();
        assert_eq!(errs.len(), 2, "{errs:?}");
        assert!(errs.iter().all(|e| e.contains("0.11.2")));
    }

    /// The committed scorecard was written before the detector and region fields (ruling B5).
    #[test]
    fn old_scorecard_entry_parses_with_defaults() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../formats/pagevision/eval/scorecard.json");
        let card: Scorecard =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let f = &card.engines[0].fingerprint;
        assert_eq!(f.layout_sha256, "");
        assert_eq!(f.candle, "");
        assert_eq!((f.rules, f.ink), (0, 0));
        assert!(card.engines[0].public.iter().all(|p| p.regions.is_none()));
    }

    /// A run that proposed no regions (a baseline, or a page the engine could not read) has no
    /// region score: an empty list is not a proposal that missed every diagram.
    #[test]
    fn run_without_regions_has_no_region_score() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = page(dir.path(), "r", false);
        let truth = dir.path().join("r.regions");
        std::fs::write(&truth, r#"{"page": "r", "diagrams": [[0, 0, 20, 20]]}"#).unwrap();
        p.regions = Some(truth);
        let m = Manifest {
            set: "public".into(),
            pages: vec![p],
        };
        let s = score(&[m], &results(&["r"]), false).unwrap();
        assert_eq!(s.public[0].regions, None);
        assert!(s.public[0].cer.is_some());
    }

    #[test]
    fn region_truth_is_scored() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = page(dir.path(), "r", false);
        let truth = dir.path().join("r.regions");
        std::fs::write(
            &truth,
            r#"{"page": "r", "units": "page pixels x0,y0,x1,y1", "diagrams": [[0, 0, 20, 20]]}"#,
        )
        .unwrap();
        p.regions = Some(truth);
        let mut r = run("r");
        r.blocks.clear();
        r.regions = vec![
            crate::RunRegion {
                kind: "drawing".into(),
                bbox: [0, 0, 500, 500],
            },
            crate::RunRegion {
                kind: "text".into(),
                bbox: [0, 500, 500, 600],
            },
        ];
        let m = Manifest {
            set: "private".into(),
            pages: vec![p],
        };
        let mut rf = results(&[]);
        rf.pages.push(r);
        let s = score(&[m], &rf, false).unwrap();
        let sums = s.private.unwrap().regions.unwrap();
        assert_eq!((sums.pages, sums.ok, sums.truth, sums.hit), (1, 1, 1, 1));
        assert_eq!((sums.split, sums.false_drawings), (0, 0));
        assert_eq!(sums.text_cov_mean, 1.0);
    }

    #[test]
    fn regions_only_run_has_no_text_metrics() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = page(dir.path(), "r", false);
        let edges = dir.path().join("r.edges");
        std::fs::write(&edges, "A --> B\n").unwrap();
        p.edges = Some(edges);
        let mut r = run("r");
        r.markdown.clear();
        r.blocks.clear();
        r.regions = vec![crate::RunRegion {
            kind: "text".into(),
            bbox: [0, 0, 1000, 1000],
        }];
        let public = Manifest {
            set: "public".into(),
            pages: vec![p.clone()],
        };
        let private = Manifest {
            set: "private".into(),
            pages: vec![PageEntry {
                id: "s".into(),
                ..p
            }],
        };
        let mut rf = results(&[]);
        rf.pages.push(r.clone());
        rf.pages.push(PageRun {
            id: "s".into(),
            ..r
        });
        let s = score(&[public, private], &rf, false).unwrap();
        let scored = &s.public[0];
        assert_eq!((scored.cer, scored.edges, scored.boxes), (None, None, None));
        assert_eq!(scored.blocks, 1);
        assert_eq!(s.private.as_ref().unwrap().read_pages, Some(0));
        let md = render_markdown(&s);
        assert!(md.contains("| r | - | - | - | - | - |"), "{md}");
        assert!(
            md.contains("| private set (1 pages) | - | - | - | - | - |"),
            "{md}"
        );
        // A reading run keeps its metrics.
        let s = score(
            &[Manifest {
                set: "private".into(),
                pages: vec![page(dir.path(), "t", false)],
            }],
            &results(&["t"]),
            false,
        )
        .unwrap();
        assert_eq!(s.private.unwrap().read_pages, Some(1));
    }

    #[test]
    fn scorecard_matches_build() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let card_path = root.join("crates/formats/pagevision/eval/scorecard.json");
        let raw = std::fs::read_to_string(&card_path)
            .unwrap_or_else(|e| panic!("{}: {e}", card_path.display()));
        let card: Scorecard = serde_json::from_str(&raw).unwrap();
        let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
        if let Err(errors) = check_fingerprints(&card, &lock) {
            panic!("scorecard is stale:\n{}", errors.join("\n"));
        }
    }
}
