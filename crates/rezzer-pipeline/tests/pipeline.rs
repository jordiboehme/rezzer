// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use rezzer_limits::Budget;
use rezzer_model::{
    Asset, Blob, Block, Completeness, Document, FormatId, Inline, LossReason, ReadOptions,
    ReadOutcome, Reader, Registry, Source, source_info,
};
use rezzer_pipeline::{Config, OutputKind, OutputSection, Pipeline, Request};

const AT: &str = "2026-10-19T08:30:00Z";

fn request() -> Request {
    Request {
        generated_at: Some(AT.to_string()),
        ..Request::default()
    }
}

/// A config that never touches the user's cache: the cache is off unless a dir is given.
fn config(cache_dir: Option<&Path>) -> Config {
    Config {
        cache: cache_dir.is_some(),
        cache_dir: cache_dir.map(Path::to_path_buf),
        ..Config::default()
    }
}

fn paragraphs(doc: &Document) -> Vec<String> {
    doc.body
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph { inlines, .. } => Some(
                inlines
                    .iter()
                    .map(|i| match i {
                        Inline::Text { text, .. } => text.as_str(),
                        _ => "",
                    })
                    .collect(),
            ),
            _ => None,
        })
        .collect()
}

/// A text reader that counts its calls and optionally panics.
struct Counting {
    calls: Arc<AtomicUsize>,
    panic: bool,
}

impl Reader for Counting {
    fn id(&self) -> &'static str {
        "counting"
    }
    fn version(&self) -> &'static str {
        "9.9.9"
    }
    fn formats(&self) -> &'static [FormatId] {
        &[FormatId::Text]
    }
    fn read(&self, src: &Source, _opts: &ReadOptions, _budget: &Budget) -> ReadOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.panic {
            panic!("boom in the reader");
        }
        let mut doc = Document::new(source_info(src));
        doc.body.push(Block::Paragraph {
            inlines: vec![Inline::Text {
                text: String::from_utf8_lossy(src.bytes()).into_owned(),
                emphasis: false,
                strong: false,
                strike: false,
            }],
            role: None,
            anchor: None,
        });
        ReadOutcome {
            blobs: Vec::new(),
            document: doc,
            completeness: Completeness::Complete,
        }
    }
}

fn counting(panic: bool) -> (Registry, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = Registry::new();
    registry.register(Box::new(Counting {
        calls: calls.clone(),
        panic,
    }));
    (registry, calls)
}

fn cache_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect()
}

#[test]
fn unknown_format_is_partial_with_unsupported_loss() {
    let pipeline = Pipeline::new(config(None), Registry::new());
    let bytes: Vec<u8> = (0..512u32).map(|i| (i * 7 % 256) as u8).collect();
    let out = pipeline.run_bytes("blob.bin", bytes, None, &request());
    assert_eq!(out.completeness, Completeness::Partial);
    let doc = out.document.expect("a document with the source facts");
    assert_eq!(doc.source.format, FormatId::Unknown);
    assert_eq!(paragraphs(&doc), ["Rezzer doesn't know this format."]);
    assert!(
        doc.losses.iter().any(|l| l.why == LossReason::Unsupported),
        "{:?}",
        doc.losses
    );
    let md = out.markdown_pages.concat();
    assert!(md.contains("Rezzer doesn't know this format."), "{md}");
    assert!(md.contains(&doc.source.sha256), "{md}");
}

#[test]
fn format_without_reader_is_partial_and_named() {
    let pipeline = Pipeline::new(config(None), Registry::new());
    let out = pipeline.run_bytes("a.pdf", b"%PDF-1.7\n%\xE2\xE3\n".to_vec(), None, &request());
    assert_eq!(out.completeness, Completeness::Partial);
    let doc = out.document.unwrap();
    assert_eq!(paragraphs(&doc), ["Rezzer can't read this format yet."]);
    let loss = doc
        .losses
        .iter()
        .find(|l| l.why == LossReason::Unsupported)
        .unwrap();
    assert_eq!(loss.what, "pdf files");
}

#[test]
fn empty_file_is_complete() {
    let pipeline = Pipeline::new(config(None), Registry::new());
    let out = pipeline.run_bytes("empty.txt", Vec::new(), None, &request());
    assert_eq!(out.completeness, Completeness::Complete);
    let doc = out.document.unwrap();
    assert_eq!(paragraphs(&doc), ["The file is empty."]);
    assert!(doc.losses.is_empty());
}

#[test]
fn panicking_reader_is_failed() {
    let (registry, calls) = counting(true);
    let pipeline = Pipeline::new(config(None), registry);
    let out = pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &request());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(out.completeness, Completeness::Failed);
    let doc = out.document.expect("the source facts survive a panic");
    let loss = doc.losses.first().expect("a loss for the panic");
    assert_eq!(loss.why, LossReason::Internal);
    assert!(
        loss.what.contains("pipeline.rs:"),
        "location: {}",
        loss.what
    );
    assert!(loss.what.contains("counting"), "{}", loss.what);
    assert!(loss.what.contains("boom in the reader"), "{}", loss.what);
    let message = out.message.unwrap();
    assert!(message.contains("counting"), "{message}");
    assert!(out.markdown_pages.concat().contains("completeness: failed"));
}

#[test]
fn cache_hit_skips_reader() {
    let dir = tempfile::tempdir().unwrap();
    let (registry, calls) = counting(false);
    let pipeline = Pipeline::new(config(Some(dir.path())), registry);
    let first = pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &request());
    let second = pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &request());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(first.markdown_pages, second.markdown_pages);
    assert_eq!(first.document, second.document);
    assert_eq!(cache_files(dir.path()).len(), 1);
}

#[test]
fn cache_hit_takes_the_path_from_the_current_source() {
    let dir = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(files.path().join("one")).unwrap();
    std::fs::create_dir_all(files.path().join("two")).unwrap();
    let a = files.path().join("one/x.txt");
    let b = files.path().join("two/x.txt");
    std::fs::write(&a, "hello\n").unwrap();
    std::fs::write(&b, "hello\n").unwrap();
    let (registry, calls) = counting(false);
    let pipeline = Pipeline::new(config(Some(dir.path())), registry);
    pipeline.run_path(&a, &request());
    let out = pipeline.run_path(&b, &request());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let doc = out.document.unwrap();
    let b = std::fs::canonicalize(&b).unwrap();
    assert_eq!(doc.source.path.as_deref(), Some(b.to_str().unwrap()));
}

#[test]
fn the_file_name_is_part_of_the_cache_key() {
    // The CSV reader names its sheet after the file: a renamed copy must not get the old name.
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::new();
    registry.register(Box::new(rezzer_read_csv::CsvReader));
    let pipeline = Pipeline::new(config(Some(dir.path())), registry);
    let bytes = b"a,b\n1,2\n3,4\n";
    pipeline.run_bytes("first.csv", bytes.to_vec(), None, &request());
    let out = pipeline.run_bytes("second.csv", bytes.to_vec(), None, &request());
    let md = out.markdown_pages.concat();
    assert!(md.contains("| second |"), "{md}");
    assert!(!md.contains("first"), "{md}");
    assert_eq!(cache_files(dir.path()).len(), 2);
}

#[test]
fn cache_off_reads_every_time() {
    let dir = tempfile::tempdir().unwrap();
    let (registry, calls) = counting(false);
    let mut cfg = config(Some(dir.path()));
    cfg.cache = false;
    let pipeline = Pipeline::new(cfg, registry);
    pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &request());
    pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &request());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(cache_files(dir.path()).is_empty());
}

#[test]
fn output_options_are_not_part_of_the_cache_key() {
    let dir = tempfile::tempdir().unwrap();
    let (registry, calls) = counting(false);
    let pipeline = Pipeline::new(config(Some(dir.path())), registry);
    pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &request());
    let later = Request {
        generated_at: Some("2027-01-01T00:00:00Z".into()),
        ..Request::default()
    };
    let out = pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &later);
    assert!(out.markdown_pages[0].contains("2027-01-01T00:00:00Z"));
    let sections = Request {
        sections: Some(vec![OutputSection::NotRead]),
        ..request()
    };
    let out = pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &sections);
    assert!(
        !out.markdown_pages.concat().contains("hello"),
        "sections still apply"
    );
    let json = Request {
        output: OutputKind::Json,
        ..request()
    };
    let out = pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &json);
    assert!(out.json.is_some());
    // Markdown, JSON and section runs share one entry: the cache holds the document.
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(cache_files(dir.path()).len(), 1);
}

#[test]
fn reading_options_are_part_of_the_cache_key() {
    let dir = tempfile::tempdir().unwrap();
    let (registry, calls) = counting(false);
    let mut cfg = config(Some(dir.path()));
    let first = Pipeline::new(cfg.clone(), registry);
    first.run_bytes("x.txt", b"hello\n".to_vec(), None, &request());
    cfg.enrich.disabled = vec!["language".into()];
    let (registry2, calls2) = counting(false);
    let second = Pipeline::new(cfg, registry2);
    second.run_bytes("x.txt", b"hello\n".to_vec(), None, &request());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(calls2.load(Ordering::SeqCst), 1, "other passes: a miss");
}

fn dir_bytes(dir: &Path) -> u64 {
    cache_files(dir)
        .iter()
        .map(|p| std::fs::metadata(p).unwrap().len())
        .sum()
}

fn set_mtime(path: &Path, t: SystemTime) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(t)
        .unwrap();
}

/// Runs `content` through the pipeline and gives the entry it wrote an explicit mtime, so
/// "oldest" does not depend on the file system's time resolution.
fn run_aged(pipeline: &Pipeline, dir: &Path, content: &str, t: SystemTime) {
    let before = cache_files(dir);
    pipeline.run_bytes("x.txt", content.as_bytes().to_vec(), None, &request());
    for path in cache_files(dir) {
        if !before.contains(&path) {
            set_mtime(&path, t);
        }
    }
}

#[test]
fn cache_is_capped_and_drops_the_oldest_entries() {
    let dir = tempfile::tempdir().unwrap();
    let (registry, calls) = counting(false);
    let probe = Pipeline::new(config(Some(dir.path())), registry);
    probe.run_bytes("x.txt", b"entry 0\n".to_vec(), None, &request());
    let one = dir_bytes(dir.path());
    assert!(one > 0);
    for path in cache_files(dir.path()) {
        std::fs::remove_file(path).unwrap();
    }

    let mut cfg = config(Some(dir.path()));
    cfg.cache_max_bytes = one * 5 / 2;
    let (registry, calls_capped) = counting(false);
    let pipeline = Pipeline::new(cfg.clone(), registry);
    let base = SystemTime::now() - Duration::from_secs(10_000);
    for i in 1..=5u64 {
        let t = base + Duration::from_secs(i * 60);
        run_aged(&pipeline, dir.path(), &format!("entry {i}\n"), t);
        assert!(
            dir_bytes(dir.path()) <= cfg.cache_max_bytes,
            "after entry {i}"
        );
    }
    assert!(cache_files(dir.path()).len() <= 2);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(calls_capped.load(Ordering::SeqCst), 5);
    // The newest entry is kept, the oldest is gone.
    pipeline.run_bytes("x.txt", b"entry 5\n".to_vec(), None, &request());
    assert_eq!(calls_capped.load(Ordering::SeqCst), 5);
    pipeline.run_bytes("x.txt", b"entry 1\n".to_vec(), None, &request());
    assert_eq!(calls_capped.load(Ordering::SeqCst), 6);
}

#[test]
fn a_cache_hit_keeps_an_entry_alive() {
    let dir = tempfile::tempdir().unwrap();
    let (registry, _) = counting(false);
    let probe = Pipeline::new(config(Some(dir.path())), registry);
    probe.run_bytes("x.txt", b"entry 0\n".to_vec(), None, &request());
    let one = dir_bytes(dir.path());
    for path in cache_files(dir.path()) {
        std::fs::remove_file(path).unwrap();
    }

    let mut cfg = config(Some(dir.path()));
    cfg.cache_max_bytes = one * 5 / 2;
    let (registry, calls) = counting(false);
    let pipeline = Pipeline::new(cfg, registry);
    let base = SystemTime::now() - Duration::from_secs(10_000);
    run_aged(&pipeline, dir.path(), "entry A\n", base);
    run_aged(
        &pipeline,
        dir.path(),
        "entry B\n",
        base + Duration::from_secs(60),
    );
    // A hit on the older entry makes it the most recently used one.
    pipeline.run_bytes("x.txt", b"entry A\n".to_vec(), None, &request());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    run_aged(&pipeline, dir.path(), "entry C\n", SystemTime::now());
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    pipeline.run_bytes("x.txt", b"entry A\n".to_vec(), None, &request());
    assert_eq!(calls.load(Ordering::SeqCst), 3, "A survived");
    pipeline.run_bytes("x.txt", b"entry B\n".to_vec(), None, &request());
    assert_eq!(calls.load(Ordering::SeqCst), 4, "B was evicted");
}

#[test]
fn page_size_is_not_part_of_the_cache_key() {
    let dir = tempfile::tempdir().unwrap();
    let (registry, calls) = counting(false);
    Pipeline::new(config(Some(dir.path())), registry).run_bytes(
        "x.txt",
        b"hello\n".to_vec(),
        None,
        &request(),
    );
    let mut cfg = config(Some(dir.path()));
    cfg.limits.page_tokens = 77;
    let (registry, calls_paged) = counting(false);
    Pipeline::new(cfg, registry).run_bytes("x.txt", b"hello\n".to_vec(), None, &request());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(calls_paged.load(Ordering::SeqCst), 0);
}

#[test]
fn a_cancelled_request_stops_the_reader_and_is_not_cached() {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::new();
    registry.register(Box::new(rezzer_read_text::TextReader));
    let pipeline = Pipeline::new(config(Some(dir.path())), registry);
    let req = request();
    req.cancel.cancel();
    let text: String = (0..1100).map(|i| format!("Paragraph {i}.\n\n")).collect();
    let out = pipeline.run_bytes("long.txt", text.into_bytes(), None, &req);
    assert_eq!(out.completeness, Completeness::Partial);
    let doc = out.document.unwrap();
    assert!(
        doc.losses.iter().any(|l| l.why == LossReason::Limit),
        "{:?}",
        doc.losses
    );
    assert!(cache_files(dir.path()).is_empty());
}

#[test]
fn a_time_cut_is_not_cached() {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::new();
    registry.register(Box::new(rezzer_read_text::TextReader));
    let mut cfg = config(Some(dir.path()));
    cfg.limits.max_time = Duration::ZERO;
    let pipeline = Pipeline::new(cfg, registry);
    let text: String = (0..1100).map(|i| format!("Paragraph {i}.\n\n")).collect();
    let out = pipeline.run_bytes("long.txt", text.into_bytes(), None, &request());
    assert_eq!(out.completeness, Completeness::Partial);
    let doc = out.document.unwrap();
    assert!(
        doc.losses.iter().any(|l| l.why == LossReason::Limit),
        "{:?}",
        doc.losses
    );
    assert!(cache_files(dir.path()).is_empty());
}

/// A reader whose own time share ran out: it notes a time stop while the budget's time is
/// not over, and gives a complete document.
struct OwnTimeShare;

impl Reader for OwnTimeShare {
    fn id(&self) -> &'static str {
        "own-time-share"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn formats(&self) -> &'static [FormatId] {
        &[FormatId::Text]
    }
    fn read(&self, src: &Source, _opts: &ReadOptions, budget: &Budget) -> ReadOutcome {
        budget.note_time_stop();
        assert!(budget.check_time().is_ok());
        ReadOutcome {
            blobs: Vec::new(),
            document: Document::new(source_info(src)),
            completeness: Completeness::Complete,
        }
    }
}

#[test]
fn a_time_stop_inside_the_limit_is_not_cached() {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::new();
    registry.register(Box::new(OwnTimeShare));
    let pipeline = Pipeline::new(config(Some(dir.path())), registry);
    let out = pipeline.run_bytes("a.txt", b"hello\n".to_vec(), None, &request());
    assert_eq!(out.completeness, Completeness::Complete);
    assert!(cache_files(dir.path()).is_empty());
}

#[test]
fn other_partial_results_are_cached() {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::new();
    registry.register(Box::new(rezzer_read_csv::CsvReader));
    let pipeline = Pipeline::new(config(Some(dir.path())), registry);
    let broken = b"a,b,c\n1,2,3\n4,5\n6,7,8\n9,10,11,12\n13,14,15\n";
    let out = pipeline.run_bytes("broken.csv", broken.to_vec(), None, &request());
    assert_eq!(out.completeness, Completeness::Partial);
    assert_eq!(cache_files(dir.path()).len(), 1);
}

#[test]
fn source_path_is_canonical_and_keeps_the_given_name() {
    let dir = tempfile::tempdir().unwrap();
    let real = std::fs::canonicalize(dir.path()).unwrap();
    std::fs::create_dir_all(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("x.txt"), "hello\n").unwrap();
    let (registry, _) = counting(false);
    let pipeline = Pipeline::new(config(None), registry);
    let out = pipeline.run_path(&dir.path().join("sub/../x.txt"), &request());
    let doc = out.document.unwrap();
    let expected = real.join("x.txt");
    assert_eq!(doc.source.path.as_deref(), Some(expected.to_str().unwrap()));
    assert_eq!(doc.source.name, "x.txt");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.path().join("x.txt"), dir.path().join("link.txt")).unwrap();
        let doc = pipeline
            .run_path(&dir.path().join("link.txt"), &request())
            .document
            .unwrap();
        assert_eq!(doc.source.name, "link.txt");
        let expected = real.join("link.txt");
        assert_eq!(doc.source.path.as_deref(), Some(expected.to_str().unwrap()));
    }
}

#[test]
fn build_id_is_set() {
    assert!(!rezzer_pipeline::BUILD_ID.is_empty());
}

#[test]
fn an_unusable_cache_never_fails_a_run() {
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("not-a-dir");
    std::fs::write(&blocker, "file in the way").unwrap();
    let (registry, calls) = counting(false);
    let pipeline = Pipeline::new(config(Some(&blocker)), registry);
    let out = pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &request());
    assert_eq!(out.completeness, Completeness::Complete);
    assert!(out.message.is_none());
    pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &request());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn corrupt_cache_entry_is_ignored_and_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let (registry, calls) = counting(false);
    let pipeline = Pipeline::new(config(Some(dir.path())), registry);
    pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &request());
    let files = cache_files(dir.path());
    assert_eq!(files.len(), 1);
    std::fs::write(&files[0], b"{ not json").unwrap();
    let out = pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &request());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(out.completeness, Completeness::Complete);
    let text = std::fs::read_to_string(&files[0]).unwrap();
    serde_json::from_str::<serde_json::Value>(&text).expect("entry rewritten as JSON");
    pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &request());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn failed_results_are_not_cached() {
    let dir = tempfile::tempdir().unwrap();
    let (registry, calls) = counting(true);
    let pipeline = Pipeline::new(config(Some(dir.path())), registry);
    pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &request());
    pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &request());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(cache_files(dir.path()).is_empty());
}

#[test]
fn a_password_skips_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let (registry, calls) = counting(false);
    let pipeline = Pipeline::new(config(Some(dir.path())), registry);
    let req = Request {
        password: Some("secret".into()),
        ..request()
    };
    pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &req);
    pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &req);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(cache_files(dir.path()).is_empty());
    assert!(!format!("{req:?}").contains("secret"));
}

#[test]
fn json_output_is_the_model() {
    let mut registry = Registry::new();
    registry.register(Box::new(rezzer_read_csv::CsvReader));
    let pipeline = Pipeline::new(config(None), registry);
    let req = Request {
        output: OutputKind::Json,
        ..request()
    };
    let out = pipeline.run_bytes("t.csv", b"a,b\n1,2\n3,4\n".to_vec(), None, &req);
    let doc: Document = serde_json::from_str(out.json.as_deref().unwrap()).unwrap();
    assert_eq!(doc.model_version, 1);
    assert_eq!(Some(doc), out.document);
    // Markdown is always rendered too, so a caller can write both.
    assert!(!out.markdown_pages.is_empty());
    assert!(request().output == OutputKind::Markdown);
    assert!(
        pipeline
            .run_bytes("t.csv", b"a,b\n1,2\n3,4\n".to_vec(), None, &request())
            .json
            .is_none()
    );
}

#[test]
fn mime_hint_reaches_detection() {
    let mut registry = Registry::new();
    registry.register(Box::new(rezzer_read_csv::CsvReader));
    registry.register(Box::new(rezzer_read_text::TextReader));
    let pipeline = Pipeline::new(config(None), registry);
    let out = pipeline.run_bytes(
        "upload",
        b"a,b\n1,2\n".to_vec(),
        Some("text/csv"),
        &request(),
    );
    assert_eq!(out.document.unwrap().source.format, FormatId::Csv);
    let out = pipeline.run_bytes("upload", b"a,b\n1,2\n".to_vec(), None, &request());
    assert_eq!(out.document.unwrap().source.format, FormatId::Text);
}

#[test]
fn pages_follow_the_configured_page_size() {
    let mut registry = Registry::new();
    registry.register(Box::new(rezzer_read_text::TextReader));
    let mut cfg = config(None);
    cfg.limits.page_tokens = 50;
    let pipeline = Pipeline::new(cfg, registry);
    let text: String = (0..40)
        .map(|i| format!("Paragraph number {i} with a few words in it.\n\n"))
        .collect();
    let out = pipeline.run_bytes("long.txt", text.into_bytes(), None, &request());
    assert!(out.markdown_pages.len() > 1, "{}", out.markdown_pages.len());
    assert!(out.markdown_pages[0].starts_with("---\n"));
}

#[test]
fn generated_at_comes_from_the_request() {
    let pipeline = Pipeline::new(config(None), Registry::new());
    let out = pipeline.run_bytes("e", Vec::new(), None, &request());
    assert!(
        out.markdown_pages[0].contains(AT),
        "{}",
        out.markdown_pages[0]
    );
}

#[test]
fn missing_path_is_failed_with_a_message() {
    let pipeline = Pipeline::new(config(None), Registry::new());
    let out = pipeline.run_path(Path::new("/nonexistent/rezzer/x.txt"), &request());
    assert_eq!(out.completeness, Completeness::Failed);
    assert!(out.document.is_none());
    assert!(out.markdown_pages.is_empty());
    assert!(out.message.unwrap().contains("not found"));
}

#[test]
fn inspect_reports_format_hash_and_reader() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.csv");
    std::fs::write(&path, "a,b\n1,2\n3,4\n").unwrap();
    let mut registry = Registry::new();
    registry.register(Box::new(rezzer_read_csv::CsvReader));
    let pipeline = Pipeline::new(config(None), registry);
    let info = pipeline.inspect(&path).unwrap();
    assert_eq!(info.name, "t.csv");
    assert_eq!(info.format, FormatId::Csv);
    assert_eq!(info.size, 12);
    assert_eq!(info.sha256.len(), 64);
    assert_eq!(info.reader.as_deref(), Some("csv"));
    let none = Pipeline::new(config(None), Registry::new());
    assert_eq!(none.inspect(&path).unwrap().reader, None);
    assert!(none.inspect(Path::new("/nonexistent/x")).is_err());
}

#[test]
fn config_defaults_when_file_missing() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config::load_from(&dir.path().join("config.toml")).unwrap();
    let default = Config::default();
    assert_eq!(cfg.limits, default.limits);
    assert_eq!(cfg.limits, rezzer_limits::Limits::default());
    assert!(cfg.cache);
    assert_eq!(cfg.cache_dir, None);
    assert_eq!(cfg.cache_max_bytes, 512 * 1024 * 1024);
    assert!(cfg.enrich.disabled.is_empty());
}

#[test]
fn config_reads_its_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let cache = dir.path().join("cache");
    std::fs::write(
        &path,
        format!(
            "cache = false\ncache_dir = {:?}\ncache_max_bytes = 4096\n\n[limits]\npage_tokens = 1000\nmax_time = 5\n\n[enrich]\ndisabled = [\"language\"]\n",
            cache.to_str().unwrap()
        ),
    )
    .unwrap();
    let cfg = Config::load_from(&path).unwrap();
    assert!(!cfg.cache);
    assert_eq!(cfg.cache_dir, Some(cache));
    assert_eq!(cfg.cache_max_bytes, 4096);
    assert_eq!(cfg.limits.page_tokens, 1000);
    assert_eq!(cfg.limits.max_time, std::time::Duration::from_secs(5));
    assert_eq!(cfg.limits.max_depth, 256);
    assert_eq!(cfg.enrich.disabled, ["language"]);
}

#[test]
fn config_unknown_key_names_file_and_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    for (text, key) in [
        ("cache = true\ncolour = \"red\"\n", "colour"),
        ("[limits]\nmax_pages = 3\n", "max_pages"),
        ("[enrich]\nskip = []\n", "skip"),
    ] {
        std::fs::write(&path, text).unwrap();
        let err = Config::load_from(&path).unwrap_err().to_string();
        assert!(err.contains(path.to_str().unwrap()), "{err}");
        assert!(err.contains(key), "{err}");
        assert!(!err.contains('\n'), "one line: {err}");
    }
}

#[test]
fn config_rejects_unknown_passes_and_relative_cache_dir() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[enrich]\ndisabled = [\"langauge\"]\n").unwrap();
    let err = Config::load_from(&path).unwrap_err().to_string();
    assert!(
        err.contains("langauge") && err.contains("language"),
        "{err}"
    );
    std::fs::write(&path, "cache_dir = \"relative/cache\"\n").unwrap();
    let err = Config::load_from(&path).unwrap_err().to_string();
    assert!(err.contains("cache_dir"), "{err}");
}

#[test]
fn config_load_uses_xdg_config_home() {
    // Only the path lookup is tested; the real home config is never read.
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        rezzer_pipeline::config_path_from(Some(dir.path().as_os_str()), None),
        Some(dir.path().join("rezzer/config.toml"))
    );
    let home = Path::new("/home/someone");
    assert_eq!(
        rezzer_pipeline::config_path_from(Some("".as_ref()), Some(home.as_os_str())),
        Some(home.join(".config/rezzer/config.toml"))
    );
    assert_eq!(
        rezzer_pipeline::config_path_from(Some("relative".as_ref()), Some(home.as_os_str())),
        Some(home.join(".config/rezzer/config.toml"))
    );
    assert_eq!(rezzer_pipeline::config_path_from(None, None), None);
}

fn panicking_detector(_: &str, _: Option<&str>, _: &[u8]) -> (FormatId, rezzer_model::Detection) {
    panic!("boom in the detector");
}

#[test]
fn a_panic_during_detection_is_an_internal_loss() {
    let dir = tempfile::tempdir().unwrap();
    let (registry, calls) = counting(false);
    let mut pipeline = Pipeline::new(config(Some(dir.path())), registry);
    pipeline.set_detector(panicking_detector);
    let out = pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &request());
    assert_eq!(out.completeness, Completeness::Failed);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let doc = out.document.expect("the source facts survive");
    assert_eq!(doc.source.size, 6);
    assert_eq!(doc.source.sha256.len(), 64);
    assert_eq!(doc.source.format, FormatId::Unknown);
    let loss = doc.losses.first().unwrap();
    assert_eq!(loss.why, LossReason::Internal);
    assert!(loss.what.contains("detection"), "{}", loss.what);
    assert!(loss.what.contains("boom in the detector"), "{}", loss.what);
    assert!(
        loss.what.contains("pipeline.rs:"),
        "location: {}",
        loss.what
    );
    assert!(out.message.unwrap().contains("boom in the detector"));
    assert!(cache_files(dir.path()).is_empty());

    let file = dir.path().join("y.txt");
    std::fs::write(&file, "hello\n").unwrap();
    assert_eq!(
        pipeline.run_path(&file, &request()).completeness,
        Completeness::Failed
    );
    let info = pipeline.inspect(&file).unwrap();
    assert_eq!(info.format, FormatId::Unknown);
    assert!(info.detection.evidence.contains("boom in the detector"));
}

#[test]
fn inspect_keeps_the_open_error_kind() {
    let dir = tempfile::tempdir().unwrap();
    let pipeline = Pipeline::new(config(None), Registry::new());
    let err = pipeline.inspect(dir.path()).unwrap_err();
    assert!(
        matches!(err, rezzer_detect::OpenError::NotAFile(_)),
        "{err:?}"
    );
    let err = pipeline.inspect(&dir.path().join("missing")).unwrap_err();
    assert!(
        matches!(err, rezzer_detect::OpenError::NotFound(_)),
        "{err:?}"
    );
    let mut cfg = config(None);
    cfg.limits.max_file_bytes = 2;
    let file = dir.path().join("big.txt");
    std::fs::write(&file, "hello").unwrap();
    let err = Pipeline::new(cfg, Registry::new())
        .inspect(&file)
        .unwrap_err();
    assert!(
        matches!(err, rezzer_detect::OpenError::TooLarge { .. }),
        "{err:?}"
    );
}

/// A text reader that returns one asset per id with a blob each, and records whether
/// extraction was asked for.
struct Pictures {
    ids: Vec<&'static str>,
    extract: Arc<AtomicUsize>,
}

impl Reader for Pictures {
    fn id(&self) -> &'static str {
        "pictures"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn formats(&self) -> &'static [FormatId] {
        &[FormatId::Text]
    }
    fn read(&self, src: &Source, opts: &ReadOptions, _budget: &Budget) -> ReadOutcome {
        if opts.extract_assets {
            self.extract.fetch_add(1, Ordering::SeqCst);
        }
        let mut doc = Document::new(source_info(src));
        let mut blobs = Vec::new();
        for id in &self.ids {
            doc.assets.push(Asset {
                id: id.to_string(),
                path: None,
                sha256: "0".repeat(64),
                mime: "image/png".to_string(),
                caption: None,
                alt: None,
                needs_vision: true,
                codes: Vec::new(),
                code_unread: false,
            });
            blobs.push(Blob {
                asset: id.to_string(),
                bytes: vec![1, 2, 3],
            });
        }
        ReadOutcome {
            blobs,
            document: doc,
            completeness: Completeness::Complete,
        }
    }
}

fn pictures(ids: &[&'static str]) -> (Registry, Arc<AtomicUsize>) {
    let extract = Arc::new(AtomicUsize::new(0));
    let mut registry = Registry::new();
    registry.register(Box::new(Pictures {
        ids: ids.to_vec(),
        extract: extract.clone(),
    }));
    (registry, extract)
}

#[test]
fn blobs_written_to_bundle_and_paths_set() {
    let (registry, extract) = pictures(&["page-1", "page-2"]);
    let pipeline = Pipeline::new(config(None), registry);
    let out = tempfile::tempdir().unwrap();
    let req = Request {
        output_dir: Some(out.path().to_path_buf()),
        ..request()
    };
    let res = pipeline.run_bytes("scan.txt", b"x".to_vec(), None, &req);
    assert_eq!(extract.load(Ordering::SeqCst), 1);
    let doc = res.document.unwrap();
    let paths: Vec<_> = doc.assets.iter().map(|a| a.path.clone().unwrap()).collect();
    assert_eq!(
        paths,
        ["scan.txt.assets/page-1.png", "scan.txt.assets/page-2.png"]
    );
    for p in &paths {
        assert_eq!(std::fs::read(out.path().join(p)).unwrap(), [1, 2, 3]);
    }
    let md = res.markdown_pages.concat();
    assert!(md.contains("scan.txt.assets/page-1.png"), "{md}");
}

#[test]
fn no_output_dir_means_no_files_and_no_extract() {
    let (registry, extract) = pictures(&["page-1"]);
    let pipeline = Pipeline::new(config(None), registry);
    let res = pipeline.run_bytes("scan.txt", b"x".to_vec(), None, &request());
    assert_eq!(extract.load(Ordering::SeqCst), 0);
    assert!(res.document.unwrap().assets[0].path.is_none());
}

#[test]
fn output_dir_read_is_not_cached() {
    let cache = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let (registry, extract) = pictures(&["page-1"]);
    let pipeline = Pipeline::new(config(Some(cache.path())), registry);
    let req = Request {
        output_dir: Some(out.path().to_path_buf()),
        ..request()
    };
    pipeline.run_bytes("scan.txt", b"x".to_vec(), None, &req);
    pipeline.run_bytes("scan.txt", b"x".to_vec(), None, &req);
    assert_eq!(extract.load(Ordering::SeqCst), 2);
    assert!(cache_files(cache.path()).is_empty());
}

#[test]
fn bundle_name_cannot_escape_output_dir() {
    assert_eq!(rezzer_pipeline::bundle_name("../x.pdf"), "x.pdf");
    assert_eq!(rezzer_pipeline::bundle_name("a/b.pdf"), "b.pdf");
    assert_eq!(rezzer_pipeline::bundle_name(".hidden"), "_hidden");
    assert_eq!(rezzer_pipeline::bundle_name("C:\\evil.pdf"), "evil.pdf");
    assert_eq!(rezzer_pipeline::bundle_name(""), "file");
    assert_eq!(rezzer_pipeline::bundle_name("a b\u{e4}.pdf"), "a_b_.pdf");
    assert_eq!(rezzer_pipeline::bundle_name(&"a".repeat(500)).len(), 120);

    let (registry, _) = pictures(&["page-1"]);
    let pipeline = Pipeline::new(config(None), registry);
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join("out");
    std::fs::create_dir(&out).unwrap();
    let req = Request {
        output_dir: Some(out.clone()),
        ..request()
    };
    pipeline.run_bytes("../x.txt", b"x".to_vec(), None, &req);
    assert!(out.join("x.txt.assets/page-1.png").exists());
    assert!(!root.path().join("x.txt.assets").exists());
}

#[test]
fn bad_asset_id_is_not_written() {
    let (registry, _) = pictures(&["../evil", "ok-1"]);
    let pipeline = Pipeline::new(config(None), registry);
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join("out");
    std::fs::create_dir(&out).unwrap();
    let req = Request {
        output_dir: Some(out.clone()),
        ..request()
    };
    let res = pipeline.run_bytes("scan.txt", b"x".to_vec(), None, &req);
    let doc = res.document.unwrap();
    assert!(doc.assets[0].path.is_none());
    assert_eq!(
        doc.assets[1].path.as_deref(),
        Some("scan.txt.assets/ok-1.png")
    );
    assert!(!root.path().join("evil.png").exists());
    assert!(!out.join("scan.txt.assets/evil.png").exists());
    assert!(doc.losses.iter().any(|l| l.why == LossReason::Internal));
}

#[cfg(unix)]
#[test]
fn symlinked_assets_folder_is_refused() {
    let (registry, _) = pictures(&["page-1"]);
    let pipeline = Pipeline::new(config(None), registry);
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join("out");
    let elsewhere = root.path().join("elsewhere");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, out.join("scan.txt.assets")).unwrap();
    let req = Request {
        output_dir: Some(out),
        ..request()
    };
    let res = pipeline.run_bytes("scan.txt", b"x".to_vec(), None, &req);
    assert_eq!(res.completeness, Completeness::Partial);
    let doc = res.document.unwrap();
    assert!(doc.assets[0].path.is_none());
    assert!(doc.losses.iter().any(|l| l.why == LossReason::Internal));
    assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn symlinked_target_file_is_not_followed() {
    let (registry, _) = pictures(&["page-1"]);
    let pipeline = Pipeline::new(config(None), registry);
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join("out");
    std::fs::create_dir_all(out.join("scan.txt.assets")).unwrap();
    let victim = root.path().join("victim");
    std::fs::write(&victim, b"keep").unwrap();
    std::os::unix::fs::symlink(&victim, out.join("scan.txt.assets/page-1.png")).unwrap();
    let req = Request {
        output_dir: Some(out.clone()),
        ..request()
    };
    let res = pipeline.run_bytes("scan.txt", b"x".to_vec(), None, &req);
    assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
    let written = out.join("scan.txt.assets/page-1.png");
    assert_eq!(std::fs::read(&written).unwrap(), [1, 2, 3]);
    assert!(!std::fs::symlink_metadata(&written).unwrap().is_symlink());
    assert_eq!(res.completeness, Completeness::Complete);
}
