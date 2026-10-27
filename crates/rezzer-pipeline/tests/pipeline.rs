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
use rezzer_pipeline::{Config, OutputKind, OutputSection, Pipeline, Request, TableStyle};

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
    // The source path is canonical, without the Windows verbatim prefix.
    let b = rezzer_pipeline::strip_verbatim(std::fs::canonicalize(&b).unwrap());
    assert_eq!(doc.source.path.as_deref(), Some(b.to_str().unwrap()));
}

// macOS and Linux only: the test plants extended attributes, which Windows does not have.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn cache_hit_takes_fresh_file_facts() {
    let dir = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(files.path().join("one")).unwrap();
    std::fs::create_dir_all(files.path().join("two")).unwrap();
    let a = files.path().join("one/x.txt");
    let b = files.path().join("two/x.txt");
    for (path, tags, secs) in [
        (&a, "Rot", 1_700_000_000),
        (&b, "Blau, Projekt", 1_800_000_000),
    ] {
        std::fs::write(path, "hello\n").unwrap();
        if let Err(error) =
            fileattrs::test_support::set_xattr_at(path, "user.xdg.tags", tags.as_bytes())
        {
            eprintln!("skipped: this file system refuses user.xdg.tags ({error})");
            return;
        }
        std::fs::File::options()
            .write(true)
            .open(path)
            .and_then(|f| f.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs)))
            .unwrap();
    }
    let (registry, calls) = counting(false);
    let pipeline = Pipeline::new(config(Some(dir.path())), registry);
    let first = pipeline.run_path(&a, &request()).document.unwrap();
    let second = pipeline.run_path(&b, &request()).document.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(first.source.file_tags, ["Rot"]);
    assert_eq!(
        first.source.modified.as_deref(),
        Some("2023-11-14T22:13:20Z")
    );
    assert_eq!(second.source.file_tags, ["Blau", "Projekt"]);
    assert_eq!(
        second.source.modified.as_deref(),
        Some("2027-01-15T08:00:00Z")
    );
    let fresh =
        rezzer_detect::open(&std::fs::canonicalize(&b).unwrap(), &Default::default()).unwrap();
    assert_eq!(second.source.created, fresh.created);
    assert_eq!(second.source.downloaded_from, fresh.downloaded_from);
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
    let records = Request {
        table_style: Some(TableStyle::Records),
        ..request()
    };
    let out = pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &records);
    assert!(
        out.markdown_pages
            .concat()
            .contains("table_style: \"records\"")
    );
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
    let real = rezzer_pipeline::strip_verbatim(std::fs::canonicalize(dir.path()).unwrap());
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
        rezzer_pipeline::config_path_from(Some(dir.path().as_os_str()), None, None),
        Some(dir.path().join("rezzer/config.toml"))
    );
    let home = Path::new("/home/someone");
    assert_eq!(
        rezzer_pipeline::config_path_from(Some("".as_ref()), Some(home.as_os_str()), None),
        Some(home.join(".config/rezzer/config.toml"))
    );
    assert_eq!(
        rezzer_pipeline::config_path_from(Some("relative".as_ref()), Some(home.as_os_str()), None),
        Some(home.join(".config/rezzer/config.toml"))
    );
    assert_eq!(rezzer_pipeline::config_path_from(None, None, None), None);
}

#[test]
fn config_path_falls_back_to_userprofile() {
    // Windows sets USERPROFILE, not HOME. Plain values, so the test means the same on every OS.
    let home = Path::new("home");
    let profile = Path::new("profile");
    let path = |xdg: Option<&str>, home: Option<&Path>, profile: Option<&Path>| {
        rezzer_pipeline::config_path_from(
            xdg.map(std::ffi::OsStr::new),
            home.map(Path::as_os_str),
            profile.map(Path::as_os_str),
        )
    };
    let in_profile = Some(profile.join(".config").join("rezzer").join("config.toml"));
    assert_eq!(path(None, None, Some(profile)), in_profile);
    assert_eq!(path(None, Some(Path::new("")), Some(profile)), in_profile);
    // HOME wins over USERPROFILE, and an absolute XDG_CONFIG_HOME over both.
    assert_eq!(
        path(None, Some(home), Some(profile)),
        Some(home.join(".config").join("rezzer").join("config.toml"))
    );
    let xdg = std::env::temp_dir();
    assert_eq!(
        path(Some(xdg.to_str().unwrap()), Some(home), Some(profile)),
        Some(xdg.join("rezzer").join("config.toml"))
    );
    // An empty USERPROFILE counts as none.
    assert_eq!(path(None, None, Some(Path::new(""))), None);
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
    bytes: Vec<u8>,
    extract: Arc<AtomicUsize>,
}

/// The start of a PNG file: enough for Rezzer to confirm the type by its signature.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

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
                bytes: self.bytes.clone(),
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
    pictures_of(ids, PNG)
}

/// A reader that gives one picture or attachment of `bytes` per id.
fn pictures_of(ids: &[&'static str], bytes: &[u8]) -> (Registry, Arc<AtomicUsize>) {
    let extract = Arc::new(AtomicUsize::new(0));
    let mut registry = Registry::new();
    registry.register(Box::new(Pictures {
        ids: ids.to_vec(),
        bytes: bytes.to_vec(),
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
        assert_eq!(std::fs::read(out.path().join(p)).unwrap(), PNG);
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

// Unix only: the test plants a symlink, which needs extra rights on Windows.
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

// Unix only: the test plants a symlink, which needs extra rights on Windows.
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
    assert_eq!(std::fs::read(&written).unwrap(), PNG);
    assert!(!std::fs::symlink_metadata(&written).unwrap().is_symlink());
    assert_eq!(res.completeness, Completeness::Complete);
}

/// The names in `dir`, sorted.
fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|d| {
            d.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn a_second_run_with_fewer_pictures_leaves_no_old_file() {
    let out = tempfile::tempdir().unwrap();
    let req = Request {
        output_dir: Some(out.path().to_path_buf()),
        ..request()
    };
    let assets = out.path().join("scan.txt.assets");
    let (registry, _) = pictures(&["page-1", "page-2", "page-3"]);
    Pipeline::new(config(None), registry).run_bytes("scan.txt", b"x".to_vec(), None, &req);
    assert_eq!(
        names_in(&assets),
        ["page-1.png", "page-2.png", "page-3.png"]
    );
    let (registry, _) = pictures(&["page-1"]);
    let res =
        Pipeline::new(config(None), registry).run_bytes("scan.txt", b"x".to_vec(), None, &req);
    assert_eq!(res.completeness, Completeness::Complete);
    assert_eq!(names_in(&assets), ["page-1.png"]);
    let (registry, _) = pictures(&[]);
    let res =
        Pipeline::new(config(None), registry).run_bytes("scan.txt", b"x".to_vec(), None, &req);
    assert_eq!(res.completeness, Completeness::Complete);
    assert!(names_in(&assets).is_empty(), "{:?}", names_in(&assets));
}

#[test]
fn attachments_get_the_extension_their_bytes_confirm() {
    let out = tempfile::tempdir().unwrap();
    let req = Request {
        output_dir: Some(out.path().to_path_buf()),
        ..request()
    };
    let assets = out.path().join("scan.txt.assets");
    // The declared type says PNG; only the bytes count (ruling P5-AJ).
    let (registry, _) = pictures_of(&["attachment-1"], b"\0\x01 not a picture, named x.png");
    let res =
        Pipeline::new(config(None), registry).run_bytes("scan.txt", b"x".to_vec(), None, &req);
    let doc = res.document.unwrap();
    assert_eq!(
        doc.assets[0].path.as_deref(),
        Some("scan.txt.assets/attachment-1.bin")
    );
    assert_eq!(names_in(&assets), ["attachment-1.bin"]);
    let pdf = b"%PDF-1.7\n";
    let (registry, _) = pictures_of(&["attachment-1", "attachment-2"], pdf);
    Pipeline::new(config(None), registry).run_bytes("scan.txt", b"x".to_vec(), None, &req);
    assert_eq!(names_in(&assets), ["attachment-1.pdf", "attachment-2.pdf"]);
    // Text and JSON, told by their content.
    let (registry, _) = pictures_of(&["attachment-1"], "Notiz \u{fc}ber x\n".as_bytes());
    Pipeline::new(config(None), registry).run_bytes("scan.txt", b"x".to_vec(), None, &req);
    assert_eq!(names_in(&assets), ["attachment-1.txt"]);
    let (registry, _) = pictures_of(&["attachment-1"], b"{\"a\": [1, 2]}");
    Pipeline::new(config(None), registry).run_bytes("scan.txt", b"x".to_vec(), None, &req);
    assert_eq!(names_in(&assets), ["attachment-1.json"]);
    // A later run removes the old names of every type Rezzer writes.
    std::fs::write(assets.join("attachment-3.png"), PNG).unwrap();
    std::fs::write(assets.join("attachment-4.docx"), b"old").unwrap();
    std::fs::write(assets.join("attachment-5.webp"), b"old").unwrap();
    std::fs::write(assets.join("attachment-6.txt"), b"old").unwrap();
    std::fs::write(assets.join("notes.txt"), b"mine").unwrap();
    std::fs::write(assets.join("notes.json"), b"mine").unwrap();
    let (registry, _) = pictures(&["attachment-1"]);
    Pipeline::new(config(None), registry).run_bytes("scan.txt", b"x".to_vec(), None, &req);
    assert_eq!(
        names_in(&assets),
        ["attachment-1.png", "notes.json", "notes.txt"]
    );
}

#[test]
fn old_files_are_removed_but_links_and_folders_are_not_followed() {
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join("out");
    let assets = out.join("scan.txt.assets");
    std::fs::create_dir_all(assets.join("sub")).unwrap();
    std::fs::write(assets.join("sub/keep.png"), b"keep").unwrap();
    std::fs::write(assets.join("page-9.png"), b"old").unwrap();
    let victim = root.path().join("victim");
    std::fs::write(&victim, b"keep").unwrap();
    // Unix only: a symlink needs extra rights on Windows. The stale picture and the folder are
    // checked on every OS.
    #[cfg(unix)]
    std::os::unix::fs::symlink(&victim, assets.join("page-8.png")).unwrap();
    let req = Request {
        output_dir: Some(out.clone()),
        ..request()
    };
    let (registry, _) = pictures(&["page-1"]);
    Pipeline::new(config(None), registry).run_bytes("scan.txt", b"x".to_vec(), None, &req);
    let expected: &[&str] = if cfg!(unix) {
        &["page-1.png", "page-8.png", "sub"]
    } else {
        &["page-1.png", "sub"]
    };
    assert_eq!(names_in(&assets), expected);
    assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
    assert_eq!(std::fs::read(assets.join("sub/keep.png")).unwrap(), b"keep");
}

// ---------------------------------------------------------------------------------------
// The table style and sheet selection

fn office(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/office")
        .join(name)
}

fn sheet_pipeline(cfg: Config) -> Pipeline {
    let mut registry = Registry::new();
    registry.register(Box::new(rezzer_read_sheet::SheetReader));
    Pipeline::new(cfg, registry)
}

/// The `### <title>` part of the Logic section, up to the next `###` heading.
fn logic_part<'a>(md: &'a str, title: &str) -> &'a str {
    let logic = &md[md.find("## Logic").expect("a Logic section")..];
    let head = format!("### {title}\n");
    let start = logic.find(&head).expect("the part");
    let rest = &logic[start + head.len()..];
    &logic[start..start + head.len() + rest.find("\n### ").unwrap_or(rest.len())]
}

#[test]
fn selection_keeps_cross_sheet_roles() {
    // Calculation!B7 and C7 are read only by Data!A4:B4. With only Calculation selected
    // they are still no results, and Calculation's reads of Data stay as written.
    let pipeline = sheet_pipeline(config(None));
    let path = office("formula-across-sheets.ods");
    let json = Request {
        output: OutputKind::Json,
        ..request()
    };
    let all = pipeline.run_path(&path, &json);
    let chosen = Request {
        sheets: vec!["calculation".into()],
        ..json.clone()
    };
    let one = pipeline.run_path(&path, &chosen);
    assert_eq!(
        one.completeness,
        Completeness::Complete,
        "{:?}",
        one.message
    );
    // The model is the full read; only the Markdown changes.
    assert_eq!(one.json, all.json);
    let all_md = all.markdown_pages.concat();
    let md = one.markdown_pages.concat();
    assert_eq!(
        logic_part(&md, "Calculation"),
        logic_part(&all_md, "Calculation")
    );
    let part = logic_part(&md, "Calculation");
    assert!(
        part.contains("| B4, C4, E4 | 3 | `=Data!A3` | Data!A3 |"),
        "{part}"
    );
    let results = &part[part.find("#### Results").expect("results")..];
    assert!(
        results.contains("| D6 |") && results.contains("| D9 |"),
        "{part}"
    );
    assert!(
        !results.contains("| B7 |") && !results.contains("| C7 |"),
        "{part}"
    );
    assert!(!md.contains("### Data\n"), "{md}");
    assert!(!md.contains("## Data\n"), "{md}");
    assert!(md.contains("## Calculation\n"), "{md}");
    assert!(md.contains("### Names\n"), "{md}");
    assert!(
        md.contains("| Data | sheet | 4 rows x 4 columns, not selected | result | Calculation |"),
        "{md}"
    );
    assert!(md.contains("sheets_selected: [\"Calculation\"]\n"), "{md}");
}

#[test]
fn unknown_sheet_fails_with_names() {
    let pipeline = sheet_pipeline(config(None));
    let req = Request {
        sheets: vec!["Data".into(), "Nope".into()],
        ..request()
    };
    let out = pipeline.run_path(&office("formula-across-sheets.ods"), &req);
    assert_eq!(out.completeness, Completeness::Failed);
    assert!(out.document.is_none());
    assert!(out.markdown_pages.is_empty());
    assert_eq!(
        out.message.as_deref(),
        Some(
            "unknown sheet \"Nope\"; the workbook has these sheets: \"indiceOrga\", \"Calculation\", \"Data\""
        )
    );
    // A file without sheets.
    let (registry, _) = counting(false);
    let text = Pipeline::new(config(None), registry);
    let out = text.run_bytes("x.txt", b"hello\n".to_vec(), None, &req);
    assert_eq!(out.completeness, Completeness::Failed);
    assert!(out.document.is_none());
    assert_eq!(
        out.message.as_deref(),
        Some("this file has no sheets to select")
    );
}

#[test]
fn config_read_table_style() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "cache = false\n\n[read]\ntable_style = \" Records \"\n",
    )
    .unwrap();
    let cfg = Config::load_from(&path).unwrap();
    assert_eq!(cfg.read.table_style, TableStyle::Records);
    assert_eq!(Config::default().read.table_style, TableStyle::Table);
    // The config is the default; a request says otherwise.
    let pipeline = sheet_pipeline(cfg);
    let file = office("table-multiple.xlsx");
    let md = pipeline.run_path(&file, &request()).markdown_pages.concat();
    assert!(md.contains("table_style: \"records\"\n"), "{md}");
    assert!(md.contains("\n- Row "), "{md}");
    let table = Request {
        table_style: Some(TableStyle::Table),
        ..request()
    };
    let md = pipeline.run_path(&file, &table).markdown_pages.concat();
    assert!(!md.contains("table_style"), "{md}");
    assert!(!md.contains("\n- Row "), "{md}");
}

#[test]
fn config_bad_table_style_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[read]\ntable_style = \"rows\"\n").unwrap();
    let err = Config::load_from(&path).unwrap_err();
    assert!(
        matches!(&err, rezzer_pipeline::ConfigError::Invalid { .. }),
        "{err:?}"
    );
    let err = err.to_string();
    assert!(err.contains(path.to_str().unwrap()), "{err}");
    assert!(err.contains("read.table_style"), "{err}");
    assert!(err.contains("rows"), "{err}");
    std::fs::write(&path, "[read]\ntable_style = 1\n").unwrap();
    let err = Config::load_from(&path).unwrap_err().to_string();
    assert!(err.contains("table_style"), "{err}");
}

#[test]
fn request_debug_shows_the_style_and_sheets() {
    let req = Request {
        table_style: Some(TableStyle::Records),
        sheets: vec!["Data".into()],
        ..Request::default()
    };
    let shown = format!("{req:?}");
    assert!(shown.contains("table_style: Some(Records)"), "{shown}");
    assert!(shown.contains("sheets: [\"Data\"]"), "{shown}");
    let d = Request::default();
    assert_eq!(d.table_style, None);
    assert!(d.sheets.is_empty());
}

#[test]
fn selection_applies_to_a_cached_document() {
    let dir = tempfile::tempdir().unwrap();
    let pipeline = sheet_pipeline(config(Some(dir.path())));
    let path = office("formula-across-sheets.ods");
    let all = pipeline.run_path(&path, &request()).markdown_pages.concat();
    assert!(all.contains("\n## Data\n"), "{all}");
    let entries = cache_files(dir.path()).len();
    assert_eq!(entries, 1);
    let one = Request {
        sheets: vec!["Data".into()],
        ..request()
    };
    let md = pipeline.run_path(&path, &one).markdown_pages.concat();
    assert_eq!(cache_files(dir.path()).len(), entries, "one cache entry");
    assert!(md.contains("\n## Data\n"), "{md}");
    assert!(!md.contains("\n## Calculation\n"), "{md}");
    assert!(md.contains("sheets_selected: [\"Data\"]\n"), "{md}");
    let unknown = Request {
        sheets: vec!["Nope".into()],
        ..request()
    };
    let out = pipeline.run_path(&path, &unknown);
    assert_eq!(out.completeness, Completeness::Failed);
    assert!(out.message.unwrap().starts_with("unknown sheet \"Nope\""));
}

// ---------------------------------------------------------------------------------------
// Writing files: the main document, its parts and the part size (decisions 8, 11 and 12)

/// A reader that makes chapters from its input `<chapters> <chars per chapter>`: a heading,
/// paragraphs and, in every third chapter, a picture with a blob whose last byte is the
/// chapter count. It reads text, CSV and
/// Markdown, so any source name reaches it.
struct Chapters;

impl Reader for Chapters {
    fn id(&self) -> &'static str {
        "chapters"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn formats(&self) -> &'static [FormatId] {
        &[FormatId::Text, FormatId::Csv, FormatId::Markdown]
    }
    fn read(&self, src: &Source, _opts: &ReadOptions, _budget: &Budget) -> ReadOutcome {
        let text = String::from_utf8_lossy(src.bytes()).into_owned();
        let mut numbers = text.split_whitespace().map(|n| n.parse::<usize>().unwrap());
        let (chapters, chars) = (numbers.next().unwrap(), numbers.next().unwrap());
        let mut doc = Document::new(source_info(src));
        let mut blobs = Vec::new();
        let sentence = "Words of a chapter that fill its pages. ";
        for c in 1..=chapters {
            doc.body.push(Block::Heading {
                level: 1,
                inlines: vec![Inline::plain(format!("Chapter {c}"))],
                anchor: None,
            });
            let mut left = chars;
            while left > 0 {
                let n = (left / sentence.len()).clamp(1, 20);
                doc.body
                    .push(Block::paragraph(vec![Inline::plain(sentence.repeat(n))]));
                left = left.saturating_sub(n * sentence.len());
            }
            if c % 3 == 1 {
                let id = format!("pic-{c}");
                doc.body.push(Block::Media {
                    asset: id.clone(),
                    alt: Some(format!("Picture {c}")),
                    anchor: None,
                });
                doc.assets.push(Asset {
                    id: id.clone(),
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
                    asset: id,
                    bytes: [PNG, &[chapters as u8]].concat(),
                });
            }
        }
        ReadOutcome {
            blobs,
            document: doc,
            completeness: Completeness::Complete,
        }
    }
}

fn chapters_pipeline(cfg: Config) -> Pipeline {
    let mut registry = Registry::new();
    registry.register(Box::new(Chapters));
    Pipeline::new(cfg, registry)
}

/// Input for [`Chapters`]: 30 chapters of 4,000 chars are far over 10,000 tokens.
const LARGE: &[u8] = b"30 4000";
/// Input for [`Chapters`] that fits one file.
const SMALL: &[u8] = b"2 600";
/// [`LARGE`] and [`SMALL`] with other content (another hash) and the same chapter titles.
const OTHER_LARGE: &[u8] = b"30 4001";
const OTHER_SMALL: &[u8] = b"2 601";

fn written(out: &Path) -> Request {
    Request {
        output_dir: Some(out.to_path_buf()),
        ..request()
    }
}

/// Q002's measure, counted here on its own (preflight F15): the chars after the closing
/// `---` line, divided by 4.
fn body_tokens(markdown: &str) -> usize {
    let rest = markdown.strip_prefix("---\n").expect("frontmatter");
    let end = rest.find("\n---\n").expect("closing ---");
    rest[end + 5..].chars().count() / 4
}

/// Every file under `dir`, without following symlinks.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        if kind.is_dir() {
            out.extend(files_under(&entry.path()));
        } else if kind.is_file() {
            out.push(entry.path());
        }
    }
    out.sort();
    out
}

fn markdown_files(dir: &Path) -> Vec<PathBuf> {
    files_under(dir)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect()
}

/// The link destinations in `markdown`: `](<dest>)` and `](dest)`.
fn link_destinations(markdown: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = markdown;
    while let Some(at) = rest.find("](") {
        let after = &rest[at + 2..];
        let dest = if let Some(inner) = after.strip_prefix('<') {
            &inner[..inner.find('>').expect("a closing >")]
        } else {
            let end = after
                .find(|c: char| c == ')' || c.is_whitespace())
                .unwrap_or(after.len());
            &after[..end]
        };
        out.push(dest.to_string());
        rest = after;
    }
    out
}

/// Whether the relative link `dest` in the file `from` names an existing entry, matching
/// every path component byte for byte (as Crystalline does), never through the file
/// system's own folding of case or normalization. The path must stay inside `root`.
fn resolves(root: &Path, from: &Path, dest: &str) -> bool {
    let path = dest.split('#').next().unwrap();
    let decoded = percent_encoding::percent_decode_str(path)
        .decode_utf8()
        .expect("a UTF-8 link");
    let mut at = from.parent().unwrap().to_path_buf();
    for part in decoded.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                assert!(at != root, "{dest} in {} leaves the folder", from.display());
                at.pop();
            }
            name => {
                let found = std::fs::read_dir(&at)
                    .unwrap()
                    .any(|e| e.unwrap().file_name().as_encoded_bytes() == name.as_bytes());
                if !found {
                    return false;
                }
                at.push(name);
            }
        }
    }
    true
}

/// Whether `dest` starts with a URL scheme (`https:`, `mailto:`), so a file name with a `:`
/// is still checked.
fn has_scheme(dest: &str) -> bool {
    let Some((scheme, _)) = dest.split_once(':') else {
        return false;
    };
    scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
}

/// Asserts that every relative link in every Markdown file under `root` resolves, and
/// returns how many there are.
fn assert_links_resolve(root: &Path) -> usize {
    let mut count = 0;
    for file in markdown_files(root) {
        let text = std::fs::read_to_string(&file).unwrap();
        for dest in link_destinations(&text) {
            if dest.is_empty() || dest.starts_with('#') || has_scheme(&dest) {
                continue;
            }
            assert!(
                resolves(root, &file, &dest),
                "{dest} in {} does not resolve",
                file.display()
            );
            count += 1;
        }
    }
    count
}

#[test]
fn small_document_is_one_file() {
    let out = tempfile::tempdir().unwrap();
    let res = chapters_pipeline(config(None)).run_bytes(
        "notes.txt",
        SMALL.to_vec(),
        None,
        &written(out.path()),
    );
    assert_eq!(res.completeness, Completeness::Complete);
    assert!(res.parts.is_empty());
    let main = out.path().join("notes.txt.md");
    assert_eq!(res.main_file.as_deref(), Some(main.as_path()));
    assert_eq!(
        std::fs::read_to_string(&main).unwrap(),
        res.markdown_pages.concat()
    );
    assert_eq!(names_in(out.path()), ["notes.txt.assets", "notes.txt.md"]);
    assert!(!out.path().join("notes.txt.parts").exists());
    assert!(assert_links_resolve(out.path()) >= 1);
}

#[test]
fn generated_large_document_writes_main_and_parts() {
    let out = tempfile::tempdir().unwrap();
    let res = chapters_pipeline(config(None)).run_bytes(
        "book.txt",
        LARGE.to_vec(),
        None,
        &written(out.path()),
    );
    assert_eq!(res.completeness, Completeness::Complete);
    assert!(res.parts.len() >= 3, "{}", res.parts.len());
    let main = std::fs::read_to_string(out.path().join("book.txt.md")).unwrap();
    assert_eq!(main, res.markdown_pages.concat());
    for part in &res.parts {
        assert!(part.path.starts_with("book.txt.parts/"), "{}", part.path);
        assert!(main.contains(&format!("]({})", part.path)), "{}", part.path);
        assert!(!part.title.is_empty() && !part.location.is_empty());
    }
    let files = markdown_files(out.path());
    assert_eq!(files.len(), res.parts.len() + 1);
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        let tokens = body_tokens(&text);
        assert!(tokens <= 10_000, "{}: {tokens}", file.display());
    }
    assert!(assert_links_resolve(out.path()) > res.parts.len());
}

#[test]
#[ignore = "needs scripts/fetch-corpus.sh"]
fn large_document_writes_main_and_parts() {
    let nist =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/pdf/nist-sp800-53r5.pdf");
    assert!(nist.exists(), "run scripts/fetch-corpus.sh first");
    let mut registry = Registry::new();
    registry.register(Box::new(rezzer_read_pdf::PdfReader));
    let out = tempfile::tempdir().unwrap();
    let res = Pipeline::new(config(None), registry).run_path(&nist, &written(out.path()));
    assert_ne!(res.completeness, Completeness::Failed, "{:?}", res.message);
    assert!(res.parts.len() > 10, "{}", res.parts.len());
    for file in markdown_files(out.path()) {
        let text = std::fs::read_to_string(&file).unwrap();
        let tokens = body_tokens(&text);
        assert!(tokens <= 10_000, "{}: {tokens}", file.display());
    }
    assert_links_resolve(out.path());
}

#[test]
fn names_stay_inside_and_links_resolve() {
    for name in ["../../x.csv", "日本", "Q3 report (final) #2 100%.csv"] {
        let root = tempfile::tempdir().unwrap();
        let out = root.path().join("a/b/out");
        std::fs::create_dir_all(&out).unwrap();
        let res =
            chapters_pipeline(config(None)).run_bytes(name, LARGE.to_vec(), None, &written(&out));
        assert_eq!(res.completeness, Completeness::Complete, "{name}");
        assert!(!res.parts.is_empty(), "{name}");
        // Nothing lands next to the output folder or above it.
        assert_eq!(names_in(root.path()), ["a"], "{name}");
        assert_eq!(names_in(&root.path().join("a")), ["b"], "{name}");
        assert_eq!(names_in(&root.path().join("a/b")), ["out"], "{name}");
        let main = res.main_file.unwrap();
        assert_eq!(main.parent(), Some(out.as_path()), "{name}");
        for part in &res.parts {
            assert!(out.join(&part.path).is_file(), "{name}: {}", part.path);
        }
        let links = assert_links_resolve(&out);
        // Every part links its main document, and the main document every part.
        assert!(links >= 2 * res.parts.len(), "{name}: {links}");
    }
}

#[test]
fn nfd_names_give_nfc_files_and_links() {
    let nfd = "Bericht A\u{0308}nderung.txt";
    let nfc = "Bericht \u{c4}nderung.txt";
    assert_eq!(rezzer_pipeline::main_name(nfd), nfc);
    assert_eq!(
        rezzer_pipeline::folder_stem(nfd),
        rezzer_pipeline::folder_stem(nfc)
    );
    let out = tempfile::tempdir().unwrap();
    let res =
        chapters_pipeline(config(None)).run_bytes(nfd, LARGE.to_vec(), None, &written(out.path()));
    assert!(!res.parts.is_empty());
    for file in files_under(out.path()) {
        let relative = file.strip_prefix(out.path()).unwrap().to_str().unwrap();
        assert!(!relative.contains('\u{0308}'), "{relative}");
    }
    let main = format!("{nfc}.md");
    assert!(names_in(out.path()).contains(&main));
    // The links use NFC too; the title keeps the name as it was given.
    for file in markdown_files(out.path()) {
        let text = std::fs::read_to_string(&file).unwrap();
        for dest in link_destinations(&text) {
            let decoded = percent_encoding::percent_decode_str(&dest)
                .decode_utf8()
                .unwrap();
            assert!(
                !decoded.contains('\u{0308}'),
                "{dest} in {}",
                file.display()
            );
        }
    }
    assert!(assert_links_resolve(out.path()) >= 2 * res.parts.len());
}

#[test]
fn similar_names_keep_their_own_bundles() {
    let out = tempfile::tempdir().unwrap();
    let names = ["a b.pdf", "a_b.pdf", "x", "x.assets", "a", "a.md"];
    let pipeline = chapters_pipeline(config(None));
    let mut results = Vec::new();
    for name in names {
        let res = pipeline.run_bytes(name, LARGE.to_vec(), None, &written(out.path()));
        assert_eq!(res.completeness, Completeness::Complete, "{name}");
        assert!(!res.parts.is_empty(), "{name}");
        results.push(res);
    }
    let mut folders = std::collections::HashSet::new();
    let mut asset_folders = std::collections::HashSet::new();
    for (name, res) in names.iter().zip(&results) {
        // Each source's files are all still there after the others were written.
        let main = res.main_file.as_ref().unwrap();
        assert_eq!(
            std::fs::read_to_string(main).unwrap(),
            res.markdown_pages.concat(),
            "{name}"
        );
        for part in &res.parts {
            assert!(
                out.path().join(&part.path).is_file(),
                "{name}: {}",
                part.path
            );
        }
        let folder = res.parts[0].path.split('/').next().unwrap().to_string();
        assert!(folders.insert(folder), "{name}");
        let doc = res.document.as_ref().unwrap();
        for asset in &doc.assets {
            let path = asset.path.as_ref().unwrap();
            assert_eq!(
                std::fs::read(out.path().join(path)).unwrap(),
                [PNG, &[30]].concat()
            );
        }
        let assets = doc.assets[0].path.as_ref().unwrap();
        assert!(asset_folders.insert(assets.split('/').next().unwrap().to_string()));
    }
    assert!(assert_links_resolve(out.path()) > 0);
}

/// A file that Rezzer's cleanup would take for one of its parts.
const PART_LIKE: &str =
    "---\ngenerated: { by: rezzer/0.1.0, at: 2026-10-19T08:30:00Z }\npart: 1\n---\n\nkeep\n";

#[test]
fn rerun_removes_stale_parts() {
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join("out");
    let pipeline = chapters_pipeline(config(None));
    let run = |input: &[u8]| {
        let res = pipeline.run_bytes("grow.txt", input.to_vec(), None, &written(&out));
        assert_eq!(res.completeness, Completeness::Complete);
        assert_links_resolve(&out);
        res
    };
    let parts_folder = out.join("grow.txt.parts");
    let part_names = |res: &rezzer_pipeline::RunResult| -> Vec<String> {
        let mut names: Vec<String> = res
            .parts
            .iter()
            .map(|p| p.path.rsplit('/').next().unwrap().to_string())
            .collect();
        names.sort();
        names
    };
    // Small: one file, no parts folder.
    assert!(run(SMALL).parts.is_empty());
    assert!(!parts_folder.exists());
    // Grown over the limit.
    let large = run(b"30 4000");
    assert_eq!(names_in(&parts_folder), part_names(&large));
    // A symlink to a file that looks like a part, and a note of the user's own. The symlink
    // is Unix only: it needs extra rights on Windows.
    let victim = root.path().join("victim.md");
    std::fs::write(&victim, PART_LIKE).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&victim, parts_folder.join("chapter-99.md")).unwrap();
    let kept: &[&str] = if cfg!(unix) {
        &["chapter-99.md", "mine.md", "notes-3.md"]
    } else {
        &["mine.md", "notes-3.md"]
    };
    std::fs::write(parts_folder.join("mine.md"), "# My own note\n").unwrap();
    // A note of the user's own with a `part:` key, as book notes have.
    std::fs::write(
        parts_folder.join("notes-3.md"),
        "---\npart: 3\n---\n\nmine\n",
    )
    .unwrap();
    // A stale part of ours with a long frontmatter before its `part:` key.
    let long = format!(
        "---\nresource: \"grow.txt\"\ntags: [\"{}\"]\ngenerated: {{ by: rezzer/0.1.0, at: 2026-10-19T08:30:00Z }}\npart: 50\n---\n\nold\n",
        "a".repeat(200_000)
    );
    std::fs::write(parts_folder.join("chapter-50.md"), long).unwrap();
    // Grown more, then shrunk to fewer parts: the old parts are gone.
    let larger = run(b"40 4000");
    assert!(larger.parts.len() > large.parts.len());
    let fewer = run(b"12 4000");
    assert!(fewer.parts.len() < large.parts.len());
    let mut expected = part_names(&fewer);
    expected.extend(kept.iter().map(|k| k.to_string()));
    expected.sort();
    assert_eq!(names_in(&parts_folder), expected);
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), PART_LIKE);
    #[cfg(unix)]
    assert!(
        std::fs::symlink_metadata(parts_folder.join("chapter-99.md"))
            .unwrap()
            .is_symlink()
    );
    // Shrunk under the limit: the parts go, the folder stays for what is not Rezzer's.
    assert!(run(SMALL).parts.is_empty());
    assert_eq!(names_in(&parts_folder), kept);
    // Without them the empty folder goes too.
    for name in kept {
        std::fs::remove_file(parts_folder.join(name)).unwrap();
    }
    run(LARGE);
    assert!(parts_folder.is_dir());
    run(SMALL);
    assert!(!parts_folder.exists());
    assert!(victim.exists());
}

#[test]
fn part_size_not_in_cache_key() {
    let dir = tempfile::tempdir().unwrap();
    let (registry, calls) = counting(false);
    Pipeline::new(config(Some(dir.path())), registry).run_bytes(
        "x.txt",
        b"hello\n".to_vec(),
        None,
        &request(),
    );
    let mut cfg = config(Some(dir.path()));
    cfg.limits.part_size = 0;
    let (registry, calls_unsplit) = counting(false);
    Pipeline::new(cfg, registry).run_bytes("x.txt", b"hello\n".to_vec(), None, &request());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(calls_unsplit.load(Ordering::SeqCst), 0);
}

#[test]
fn inline_size_not_in_cache_key() {
    let dir = tempfile::tempdir().unwrap();
    let (registry, calls) = counting(false);
    Pipeline::new(config(Some(dir.path())), registry).run_bytes(
        "x.txt",
        b"hello\n".to_vec(),
        None,
        &request(),
    );
    let mut cfg = config(Some(dir.path()));
    cfg.limits.max_inline_bytes = 4096;
    let (registry, calls_again) = counting(false);
    Pipeline::new(cfg, registry).run_bytes("x.txt", b"hello\n".to_vec(), None, &request());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(calls_again.load(Ordering::SeqCst), 0);
}

#[test]
fn config_inline_size_and_bounds() {
    let limits = Config::default().limits;
    assert_eq!(limits.max_inline_bytes, 32 << 20);
    assert_eq!(limits.inline_bytes(), 32 << 20);
    let small = rezzer_limits::Limits {
        max_file_bytes: 1000,
        ..rezzer_limits::Limits::default()
    };
    assert_eq!(small.inline_bytes(), 1000);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[limits]\nmax_inline_bytes = 4096\n").unwrap();
    assert_eq!(
        Config::load_from(&path).unwrap().limits.max_inline_bytes,
        4096
    );
    for bad in ["0", "1073741825"] {
        std::fs::write(&path, format!("[limits]\nmax_inline_bytes = {bad}\n")).unwrap();
        let err = Config::load_from(&path).unwrap_err().to_string();
        assert!(
            err.ends_with("limits.max_inline_bytes must be between 1 and 1073741824"),
            "{err}"
        );
    }
}

#[test]
fn config_part_size_and_bounds() {
    assert_eq!(Config::default().limits.part_size, 10_000);
    assert_eq!(rezzer_pipeline::check_part_size(0), Ok(()));
    assert_eq!(rezzer_pipeline::check_part_size(2_500), Ok(()));
    assert_eq!(
        rezzer_pipeline::check_part_size(2_499),
        Err("part_size must be 0 or at least 2500".to_string())
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    for (value, expected) in [(5_000, 5_000), (0, 0)] {
        std::fs::write(&path, format!("[limits]\npart_size = {value}\n")).unwrap();
        assert_eq!(Config::load_from(&path).unwrap().limits.part_size, expected);
    }
    std::fs::write(&path, "[limits]\npart_size = 100\n").unwrap();
    let err = Config::load_from(&path).unwrap_err().to_string();
    assert!(err.contains(path.to_str().unwrap()), "{err}");
    assert!(
        err.ends_with("limits.part_size must be 0 or at least 2500"),
        "{err}"
    );

    // From the config, and from the request over the config.
    let out = tempfile::tempdir().unwrap();
    let mut cfg = config(None);
    cfg.limits.part_size = 0;
    let one = chapters_pipeline(cfg).run_bytes("a.txt", LARGE.to_vec(), None, &written(out.path()));
    assert!(one.parts.is_empty());
    let req = Request {
        part_size: Some(2_500),
        ..written(out.path())
    };
    let pipeline = chapters_pipeline(config(None));
    let small_parts = pipeline.run_bytes("b.txt", LARGE.to_vec(), None, &req);
    let default_parts = pipeline.run_bytes("c.txt", LARGE.to_vec(), None, &written(out.path()));
    assert!(small_parts.parts.len() > default_parts.parts.len());
    for file in markdown_files(&out.path().join("b.txt.parts")) {
        let tokens = body_tokens(&std::fs::read_to_string(&file).unwrap());
        assert!(tokens <= 2_500, "{}: {tokens}", file.display());
    }
    let bad = Request {
        part_size: Some(100),
        ..written(out.path())
    };
    let res = pipeline.run_bytes("d.txt", LARGE.to_vec(), None, &bad);
    assert_eq!(res.completeness, Completeness::Failed);
    assert_eq!(
        res.message.as_deref(),
        Some("part_size must be 0 or at least 2500")
    );
    assert!(!out.path().join("d.txt.md").exists());
}

#[test]
fn json_is_written_next_to_the_main_document() {
    let out = tempfile::tempdir().unwrap();
    let req = Request {
        output: OutputKind::Json,
        ..written(out.path())
    };
    let res = chapters_pipeline(config(None)).run_bytes("a.md", LARGE.to_vec(), None, &req);
    let json = std::fs::read_to_string(out.path().join("a.md.json")).unwrap();
    assert!(json.ends_with("}\n"));
    serde_json::from_str::<Document>(&json).unwrap();
    assert!(!res.parts.is_empty());
    // A name ending in .md gets the hash, so its parts never meet a source named `a`.
    assert!(
        res.parts[0].path.starts_with("a.md-") && res.parts[0].path.contains(".parts/"),
        "{}",
        res.parts[0].path
    );
}

#[test]
fn a_write_error_is_a_failed_result() {
    let out = tempfile::tempdir().unwrap();
    std::fs::create_dir(out.path().join("blocked.txt.md")).unwrap();
    let res = chapters_pipeline(config(None)).run_bytes(
        "blocked.txt",
        SMALL.to_vec(),
        None,
        &written(out.path()),
    );
    assert_eq!(res.completeness, Completeness::Failed);
    let message = res.message.unwrap();
    assert!(message.starts_with("cannot write "), "{message}");
    assert!(message.contains("blocked.txt.md"), "{message}");
}

#[test]
fn main_name_and_folder_stem_follow_decision_8() {
    use rezzer_pipeline::{folder_stem, main_name};
    let hashed = |name: &str, base: &str| {
        let got = main_name(name);
        let suffix = got.strip_prefix(base).unwrap_or_else(|| panic!("{got}"));
        assert_eq!(suffix.len(), 9, "{got}");
        assert!(suffix.starts_with('-'), "{got}");
        assert!(suffix[1..].bytes().all(|b| b.is_ascii_hexdigit()), "{got}");
    };
    assert_eq!(main_name("budget.xlsx"), "budget.xlsx");
    assert_eq!(main_name("../dir/x.pdf"), "x.pdf");
    assert_eq!(main_name("C:\\dir\\x.pdf"), "x.pdf");
    assert_eq!(main_name(""), "file");
    assert_eq!(main_name("a/.."), "file");
    assert_eq!(
        main_name("Q3 report (final) #2.pdf"),
        "Q3 report (final) #2.pdf"
    );
    assert_eq!(main_name("indexes.pdf"), "indexes.pdf");
    hashed("日本", "日本");
    hashed("...", "_..");
    hashed(".env", "_env");
    hashed(".env.md", "_env.md");
    hashed("index", "index");
    hashed("LOG", "LOG");
    hashed("Index.PDF", "Index.PDF");
    hashed("log.txt", "log.txt");
    hashed("a\nb\u{85}.txt", "a_b_.txt");
    assert_ne!(main_name("a\nb.txt"), main_name("a\tb.txt"));
    // Within the bound the splitter counts with, and the file system's 255 bytes.
    for name in [
        "a".repeat(500),
        "日".repeat(100),
        format!("{}\n", "b".repeat(300)),
        "x".to_string(),
        "日本".to_string(),
        "क़".repeat(30),
    ] {
        let got = main_name(&name);
        assert!(got.len() <= rezzer_render::MAIN_NAME_MAX, "{got}");
        assert!(
            got.chars().count() <= name.chars().count().max(4) + 9,
            "{got}"
        );
        assert!(got.is_char_boundary(got.len()));
    }
    assert_eq!(folder_stem("x.pdf"), "x.pdf");
    assert_eq!(folder_stem("a_b.pdf"), "a_b.pdf");
    assert!(folder_stem("a b.pdf").starts_with("a_b.pdf-"));
    assert_ne!(folder_stem("日本.pdf"), folder_stem("中国.pdf"));
    for name in ["a.md", "a.MD", "x.assets", "a.json", "x.parts"] {
        assert_eq!(folder_stem(name).len(), name.len() + 9, "{name}");
    }
    assert_eq!(folder_stem("../x.txt"), "x.txt");
    assert_eq!(rezzer_pipeline::parts_folder("x.txt"), "x.txt.parts");
}

#[test]
fn assets_cleanup_keeps_markdown() {
    // An assets folder may share its name with another source's parts folder (preflight
    // F31): its cleanup never removes Markdown.
    let out = tempfile::tempdir().unwrap();
    let assets = out.path().join("scan.txt.assets");
    std::fs::create_dir_all(&assets).unwrap();
    std::fs::write(
        assets.join("chapter-1.md"),
        "---\ngenerated: { by: rezzer/0.1.0, at: 2026-10-19T08:30:00Z }\npart: 1\n---\n",
    )
    .unwrap();
    std::fs::write(assets.join("page-9.png"), b"old").unwrap();
    let (registry, _) = pictures(&["page-1"]);
    Pipeline::new(config(None), registry).run_bytes(
        "scan.txt",
        b"x".to_vec(),
        None,
        &written(out.path()),
    );
    assert_eq!(names_in(&assets), ["chapter-1.md", "page-1.png"]);
}

/// The names of every file and folder under `dir`, without following symlinks.
fn all_names(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        out.push(entry.file_name().to_string_lossy().into_owned());
        if entry.file_type().unwrap().is_dir() {
            out.extend(all_names(&entry.path()));
        }
    }
    out
}

#[test]
fn a_source_writes_into_its_own_folder() {
    // Ruling P5-AA: the parts and assets folders never take the source's own name.
    for name in ["notes.txt", "book.markdown", "README"] {
        let src = tempfile::tempdir().unwrap();
        let file = src.path().join(name);
        std::fs::write(&file, LARGE).unwrap();
        let pipeline = chapters_pipeline(config(None));
        let first = pipeline.run_path(&file, &written(src.path()));
        assert_eq!(
            first.completeness,
            Completeness::Complete,
            "{name}: {:?}",
            first.message
        );
        assert!(!first.parts.is_empty(), "{name}");
        assert_eq!(std::fs::read(&file).unwrap(), LARGE, "{name}");
        assert_eq!(
            first.main_file.as_deref(),
            Some(src.path().join(format!("{name}.md")).as_path())
        );
        assert!(
            first.parts[0].path.starts_with(&format!("{name}.parts/")),
            "{name}"
        );
        let doc = first.document.as_ref().unwrap();
        let pictures: Vec<_> = doc.assets.iter().map(|a| a.path.clone().unwrap()).collect();
        assert!(
            pictures[0].starts_with(&format!("{name}.assets/")),
            "{name}"
        );
        assert_links_resolve(src.path());
        // A rerun gives the same paths.
        let again = pipeline.run_path(&file, &written(src.path()));
        assert_eq!(again.parts, first.parts, "{name}");
        let doc = again.document.as_ref().unwrap();
        let again_pictures: Vec<_> = doc.assets.iter().map(|a| a.path.clone().unwrap()).collect();
        assert_eq!(again_pictures, pictures, "{name}");
        assert_eq!(std::fs::read(&file).unwrap(), LARGE, "{name}");
    }
}

#[test]
fn a_file_at_a_part_path_that_is_not_ours_fails_the_run() {
    // Ruling P5-AB: never overwrite a file Rezzer did not write.
    let probe = tempfile::tempdir().unwrap();
    let pipeline = chapters_pipeline(config(None));
    let parts = pipeline
        .run_bytes("notes.txt", LARGE.to_vec(), None, &written(probe.path()))
        .parts;
    let first = parts[0].path.clone();
    let root = tempfile::tempdir().unwrap();
    let victim = root.path().join("victim.md");
    std::fs::write(&victim, PART_LIKE).unwrap();
    // The symlink case is Unix only: a symlink needs extra rights on Windows.
    let cases: &[(&str, bool)] = if cfg!(unix) {
        &[("a note", false), ("a symlink", true)]
    } else {
        &[("a note", false)]
    };
    for &(what, plant) in cases {
        let out = root.path().join(format!("out-{plant}"));
        let target = out.join(&first);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        if plant {
            #[cfg(unix)]
            std::os::unix::fs::symlink(&victim, &target).unwrap();
        } else {
            std::fs::write(&target, "# My own introduction\n").unwrap();
        }
        let res = pipeline.run_bytes("notes.txt", LARGE.to_vec(), None, &written(&out));
        assert_eq!(res.completeness, Completeness::Failed, "{what}");
        let message = res.message.unwrap();
        assert!(message.contains(&first), "{what}: {message}");
        assert!(
            message.contains("is not a part Rezzer wrote"),
            "{what}: {message}"
        );
        // The read itself worked, so its document stays in the result.
        assert!(res.document.is_some(), "{what}");
        // Nothing else of the run was written, and the file in the way is untouched.
        assert!(!out.join("notes.txt.md").exists(), "{what}");
        assert_eq!(names_in(target.parent().unwrap()).len(), 1, "{what}");
        if plant {
            assert!(std::fs::symlink_metadata(&target).unwrap().is_symlink());
        } else {
            assert_eq!(
                std::fs::read_to_string(&target).unwrap(),
                "# My own introduction\n"
            );
        }
    }
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), PART_LIKE);
    // A part Rezzer wrote at that path is replaced as before.
    let res = pipeline.run_bytes("notes.txt", LARGE.to_vec(), None, &written(probe.path()));
    assert_eq!(res.completeness, Completeness::Complete);
}

#[test]
fn dot_names_are_not_hidden() {
    // Ruling P5-Z: Crystalline skips hidden files, so no name starts with a dot.
    for name in [".env.md", ".env", "..."] {
        let out = tempfile::tempdir().unwrap();
        let res = chapters_pipeline(config(None)).run_bytes(
            name,
            LARGE.to_vec(),
            None,
            &written(out.path()),
        );
        assert_eq!(res.completeness, Completeness::Complete, "{name}");
        assert!(!res.parts.is_empty(), "{name}");
        for written in all_names(out.path()) {
            assert!(!written.starts_with('.'), "{name}: {written}");
        }
        assert!(
            assert_links_resolve(out.path()) >= 2 * res.parts.len(),
            "{name}"
        );
    }
}

// Unix only: the test plants a symlink, which needs extra rights on Windows.
#[cfg(unix)]
#[test]
fn a_symlinked_parts_folder_is_never_followed() {
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join("out");
    let elsewhere = root.path().join("elsewhere");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(elsewhere.join("chapter-1.md"), PART_LIKE).unwrap();
    std::os::unix::fs::symlink(&elsewhere, out.join("notes.txt.parts")).unwrap();
    let pipeline = chapters_pipeline(config(None));
    // A split run refuses it.
    let res = pipeline.run_bytes("notes.txt", LARGE.to_vec(), None, &written(&out));
    assert_eq!(res.completeness, Completeness::Failed);
    assert!(
        res.message
            .as_deref()
            .unwrap()
            .contains("not a plain directory"),
        "{:?}",
        res.message
    );
    // A run that fits does not clean up through it, and leaves the link.
    let res = pipeline.run_bytes("notes.txt", SMALL.to_vec(), None, &written(&out));
    assert_eq!(res.completeness, Completeness::Complete);
    assert_eq!(names_in(&elsewhere), ["chapter-1.md"]);
    assert!(
        std::fs::symlink_metadata(out.join("notes.txt.parts"))
            .unwrap()
            .is_symlink()
    );
    // Also a link to an empty folder stays.
    std::fs::remove_file(elsewhere.join("chapter-1.md")).unwrap();
    pipeline.run_bytes("notes.txt", SMALL.to_vec(), None, &written(&out));
    assert!(out.join("notes.txt.parts").exists());
    assert!(elsewhere.is_dir());
}

#[test]
fn the_main_document_never_replaces_a_file_rezzer_did_not_write() {
    // Ruling P5-AG: a hand-written file, a symlink or a folder at the main document's path
    // stops the run before anything is written, and stays as it is.
    let root = tempfile::tempdir().unwrap();
    let victim = root.path().join("victim");
    std::fs::write(&victim, "keep").unwrap();
    let pipeline = chapters_pipeline(config(None));
    for (what, input) in [("small", SMALL), ("large", LARGE)] {
        let out = root.path().join(format!("out-{what}"));
        std::fs::create_dir_all(&out).unwrap();
        // The symlink is Unix only: it needs extra rights on Windows. The note and the folder
        // are checked on every OS.
        #[cfg(unix)]
        std::os::unix::fs::symlink(&victim, out.join("link.txt.md")).unwrap();
        std::fs::write(out.join("x.txt.md"), "# My own notes\n").unwrap();
        std::fs::create_dir(out.join("dir.txt.md")).unwrap();
        let names: &[&str] = if cfg!(unix) {
            &["link.txt", "x.txt", "dir.txt"]
        } else {
            &["x.txt", "dir.txt"]
        };
        for &name in names {
            let res = pipeline.run_bytes(name, input.to_vec(), None, &written(&out));
            assert_eq!(res.completeness, Completeness::Failed, "{what} {name}");
            let message = res.message.unwrap();
            assert!(message.contains(&format!("{name}.md")), "{message}");
            assert!(
                message.contains("is not a file Rezzer wrote"),
                "{what} {name}: {message}"
            );
            assert!(res.document.is_some(), "{what} {name}");
        }
        let expected: &[&str] = if cfg!(unix) {
            &["dir.txt.md", "link.txt.md", "x.txt.md"]
        } else {
            &["dir.txt.md", "x.txt.md"]
        };
        assert_eq!(names_in(&out), expected, "{what}");
        assert_eq!(
            std::fs::read_to_string(out.join("x.txt.md")).unwrap(),
            "# My own notes\n"
        );
        #[cfg(unix)]
        assert!(
            std::fs::symlink_metadata(out.join("link.txt.md"))
                .unwrap()
                .is_symlink()
        );
    }
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
}

#[test]
fn a_rerun_of_the_same_source_replaces_its_files() {
    let out = tempfile::tempdir().unwrap();
    let pipeline = chapters_pipeline(config(None));
    let json = |dir: &Path| Request {
        output: OutputKind::Json,
        ..written(dir)
    };
    for input in [SMALL, LARGE, SMALL] {
        let res = pipeline.run_bytes("notes.txt", input.to_vec(), None, &json(out.path()));
        assert_eq!(
            res.completeness,
            Completeness::Complete,
            "{:?}",
            res.message
        );
        let main = std::fs::read_to_string(out.path().join("notes.txt.md")).unwrap();
        assert!(main.starts_with("---\ntype: source\n"), "{main}");
        assert!(out.path().join("notes.txt.json").is_file());
    }
}

#[test]
fn output_of_another_source_stops_the_run() {
    // Ruling P5-AG: `Note.txt` and then `note.txt` with other content into one folder on a
    // file system that ignores case. The clash is made with a file that has another
    // `resource`, so the test runs on any file system.
    let pipeline = chapters_pipeline(config(None));
    let json = |dir: &Path| Request {
        output: OutputKind::Json,
        ..written(dir)
    };
    for (what, input, other_input) in [("small", SMALL, OTHER_SMALL), ("large", LARGE, OTHER_LARGE)]
    {
        let other = tempfile::tempdir().unwrap();
        pipeline.run_bytes("Note.txt", other_input.to_vec(), None, &json(other.path()));
        let out = tempfile::tempdir().unwrap();
        // The main document of `Note.txt` at the path of `note.txt`.
        std::fs::copy(
            other.path().join("Note.txt.md"),
            out.path().join("note.txt.md"),
        )
        .unwrap();
        let before = snapshot_files(out.path());
        let res = pipeline.run_bytes("note.txt", input.to_vec(), None, &json(out.path()));
        assert_eq!(res.completeness, Completeness::Failed, "{what}");
        let message = res.message.unwrap();
        assert!(message.contains("note.txt.md"), "{what}: {message}");
        assert!(message.contains("\"Note.txt\""), "{what}: {message}");
        assert!(message.contains("\"note.txt\""), "{what}: {message}");
        assert!(message.contains("letter case"), "{what}: {message}");
        assert_eq!(snapshot_files(out.path()), before, "{what}");
        // The JSON of `Note.txt` at the JSON path of `note.txt`.
        std::fs::remove_file(out.path().join("note.txt.md")).unwrap();
        std::fs::copy(
            other.path().join("Note.txt.json"),
            out.path().join("note.txt.json"),
        )
        .unwrap();
        let res = pipeline.run_bytes("note.txt", input.to_vec(), None, &json(out.path()));
        assert_eq!(res.completeness, Completeness::Failed, "{what}");
        let message = res.message.unwrap();
        assert!(message.contains("note.txt.json"), "{what}: {message}");
        assert!(message.contains("\"Note.txt\""), "{what}: {message}");
        assert_eq!(names_in(out.path()), ["note.txt.json"], "{what}");
        // A JSON of the user's own stops it too.
        std::fs::write(out.path().join("note.txt.json"), "{\"mine\": true}\n").unwrap();
        let res = pipeline.run_bytes("note.txt", input.to_vec(), None, &json(out.path()));
        assert_eq!(res.completeness, Completeness::Failed, "{what}");
        assert!(
            res.message.unwrap().contains("is not a file Rezzer wrote"),
            "{what}"
        );
        // Without JSON output the JSON path is not touched, so the run works.
        let res = pipeline.run_bytes("note.txt", input.to_vec(), None, &written(out.path()));
        assert_eq!(res.completeness, Completeness::Complete, "{what}");
        assert_eq!(
            std::fs::read_to_string(out.path().join("note.txt.json")).unwrap(),
            "{\"mine\": true}\n"
        );
    }
}

/// `path` as a path relative to the working directory, built with `..` up to the root.
#[cfg(unix)]
fn relative_to_cwd(path: &Path) -> PathBuf {
    let cwd = std::env::current_dir().unwrap();
    let mut rel = PathBuf::new();
    for _ in cwd.components().skip(1) {
        rel.push("..");
    }
    rel.join(path.strip_prefix("/").unwrap())
}

#[test]
fn one_file_by_any_path_or_a_copy_is_the_same_source() {
    // Ruling P5-AG refined: the same canonical path or the same content is the same source.
    let root = tempfile::tempdir().unwrap();
    // The form Rezzer writes: canonical, without the Windows verbatim prefix.
    let real = rezzer_pipeline::strip_verbatim(std::fs::canonicalize(root.path()).unwrap());
    let src = real.join("src");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    let file = src.join("report.txt");
    std::fs::write(&file, LARGE).unwrap();
    let out = real.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let pipeline = chapters_pipeline(config(None));
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut paths = vec![
        ("absolute", file.clone()),
        ("dot-dot", src.join("sub/../report.txt")),
    ];
    // Unix only: on Windows the temp folder and the working directory can be on different
    // drives, so there is no relative path between them.
    #[cfg(unix)]
    paths.insert(1, ("relative", relative_to_cwd(&file)));
    for (what, path) in paths {
        let res = pipeline.run_path(&path, &written(&out));
        assert_eq!(
            res.completeness,
            Completeness::Complete,
            "{what}: {:?}",
            res.message
        );
    }
    // A symlink keeps its own name, so its `resource` differs. Its output names hold the
    // output of the file itself, and the file was edited since: only the canonical path
    // tells that it is the same source. Unix only: a symlink needs extra rights on Windows.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&file, src.join("link.txt")).unwrap();
        let link_out = real.join("link-out");
        std::fs::create_dir_all(&link_out).unwrap();
        for name in names_in(&out) {
            let from = out.join(&name);
            if from.is_file() {
                let to = link_out.join(name.replacen("report.txt", "link.txt", 1));
                std::fs::copy(&from, &to).unwrap();
            }
        }
        std::fs::write(&file, OTHER_LARGE).unwrap();
        let res = pipeline.run_path(&src.join("link.txt"), &written(&link_out));
        assert_eq!(
            res.completeness,
            Completeness::Complete,
            "{:?}",
            res.message
        );
        std::fs::write(&file, LARGE).unwrap();
    }
    // A copy with the same content at another path, and the original moved away.
    let copy = real.join("elsewhere");
    std::fs::create_dir_all(&copy).unwrap();
    std::fs::copy(&file, copy.join("report.txt")).unwrap();
    std::fs::remove_file(&file).unwrap();
    let res = pipeline.run_path(&copy.join("report.txt"), &written(&out));
    assert_eq!(
        res.completeness,
        Completeness::Complete,
        "{:?}",
        res.message
    );
    // Another `report.txt` with other content stops the run and names both.
    let b = real.join("b");
    std::fs::create_dir_all(&b).unwrap();
    std::fs::write(b.join("report.txt"), OTHER_LARGE).unwrap();
    let before = snapshot_files(&out);
    let res = pipeline.run_path(&b.join("report.txt"), &written(&out));
    assert_eq!(res.completeness, Completeness::Failed);
    let message = res.message.unwrap();
    // Quoted as the frontmatter quotes it, a backslash doubled (Windows).
    let quoted = |p: PathBuf| format!("\"{}\"", p.display().to_string().replace('\\', "\\\\"));
    let first = quoted(copy.join("report.txt"));
    let second = quoted(b.join("report.txt"));
    assert!(message.contains(&first), "{message}");
    assert!(message.contains(&second), "{message}");
    assert_eq!(snapshot_files(&out), before);
}

#[test]
fn a_part_of_another_source_stops_the_run_and_is_never_removed() {
    let pipeline = chapters_pipeline(config(None));
    let other = tempfile::tempdir().unwrap();
    let first = pipeline.run_bytes(
        "Note.txt",
        OTHER_LARGE.to_vec(),
        None,
        &written(other.path()),
    );
    let part = first.parts[0].path.clone();
    let out = tempfile::tempdir().unwrap();
    let ours = pipeline.run_bytes("note.txt", LARGE.to_vec(), None, &written(out.path()));
    assert_eq!(ours.completeness, Completeness::Complete);
    let folder = out.path().join("note.txt.parts");
    // A part of `Note.txt` at a path this run writes stops it.
    let target = out.path().join(part.replacen("Note.txt", "note.txt", 1));
    std::fs::copy(other.path().join(&part), &target).unwrap();
    let res = pipeline.run_bytes("note.txt", LARGE.to_vec(), None, &written(out.path()));
    assert_eq!(res.completeness, Completeness::Failed);
    let message = res.message.unwrap();
    assert!(message.contains("\"Note.txt\""), "{message}");
    // A part of `Note.txt` that this run does not write stays when the run cleans up.
    std::fs::rename(&target, folder.join("chapter-99.md")).unwrap();
    let res = pipeline.run_bytes("note.txt", SMALL.to_vec(), None, &written(out.path()));
    assert_eq!(
        res.completeness,
        Completeness::Complete,
        "{:?}",
        res.message
    );
    assert_eq!(names_in(&folder), ["chapter-99.md"]);
}

#[test]
fn hostile_names_are_written_end_to_end() {
    let long = format!("{}.txt", "a".repeat(500));
    let wide = "日".repeat(100);
    #[cfg_attr(windows, allow(unused_mut))]
    let mut names = vec![
        "/abs/dir/report.txt",
        "a\0b.txt",
        long.as_str(),
        wide.as_str(),
        "C:\\dir\\x.txt",
    ];
    // Known issue, fixed before the release: the main document keeps the characters Windows
    // reserves (`:` makes an NTFS alternate data stream, `* ? " < > |` and device names such
    // as CON cannot be written), so on Windows a:b.txt is left out until the naming fix.
    #[cfg(not(windows))]
    names.push("a:b.txt");
    for name in names {
        let root = tempfile::tempdir().unwrap();
        let out = root.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        let res =
            chapters_pipeline(config(None)).run_bytes(name, LARGE.to_vec(), None, &written(&out));
        assert_eq!(
            res.completeness,
            Completeness::Complete,
            "{name:?}: {:?}",
            res.message
        );
        assert!(!res.parts.is_empty(), "{name:?}");
        assert_eq!(names_in(root.path()), ["out"], "{name:?}");
        for written in all_names(&out) {
            assert!(written.len() <= 255, "{name:?}: {written}");
            assert!(!written.contains('\0'), "{name:?}");
        }
        assert!(
            assert_links_resolve(&out) >= 2 * res.parts.len(),
            "{name:?}"
        );
    }
}

/// The bytes of every file under `dir`, by path.
fn snapshot_files(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    files_under(dir)
        .into_iter()
        .map(|p| {
            let bytes = std::fs::read(&p).unwrap();
            (p, bytes)
        })
        .collect()
}

#[test]
fn a_run_that_fails_the_part_check_changes_nothing() {
    // A failed run leaves the old pictures and parts as they were, so every old link still
    // resolves: the part paths are checked before a picture is written, and stale pictures
    // go only after every file is written.
    let out = tempfile::tempdir().unwrap();
    let pipeline = chapters_pipeline(config(None));
    let first = pipeline.run_bytes("notes.txt", LARGE.to_vec(), None, &written(out.path()));
    assert_eq!(first.completeness, Completeness::Complete);
    let in_the_way = out.path().join(&first.parts[0].path);
    std::fs::write(&in_the_way, "# My own start\n").unwrap();
    let before = snapshot_files(out.path());
    // Fewer chapters: fewer pictures and parts, so a working run would remove some.
    let res = pipeline.run_bytes("notes.txt", b"12 4000".to_vec(), None, &written(out.path()));
    assert_eq!(res.completeness, Completeness::Failed);
    assert!(
        res.message
            .as_deref()
            .unwrap()
            .contains("is not a part Rezzer wrote"),
        "{:?}",
        res.message
    );
    assert_eq!(snapshot_files(out.path()), before);
    assert!(names_in(&out.path().join("notes.txt.assets")).len() > 4);
    assert_links_resolve(out.path());
}

#[test]
fn a_write_error_keeps_the_old_pictures() {
    // Stale pictures go only after every file is written.
    let out = tempfile::tempdir().unwrap();
    let pipeline = chapters_pipeline(config(None));
    pipeline.run_bytes("notes.txt", LARGE.to_vec(), None, &written(out.path()));
    let assets = out.path().join("notes.txt.assets");
    let pictures = names_in(&assets);
    assert_eq!(pictures.len(), 10);
    // The main document cannot be written: a folder is in the way.
    let main = out.path().join("notes.txt.md");
    std::fs::remove_file(&main).unwrap();
    std::fs::create_dir(&main).unwrap();
    let res = pipeline.run_bytes("notes.txt", b"12 4000".to_vec(), None, &written(out.path()));
    assert_eq!(res.completeness, Completeness::Failed);
    assert_eq!(names_in(&assets), pictures);
}

#[test]
fn the_assets_cleanup_keeps_files_a_user_put_there() {
    // Rulings P5-AC and P5-AK: only names Rezzer gives pictures and attachments are removed.
    let out = tempfile::tempdir().unwrap();
    let assets = out.path().join("scan.txt.assets");
    std::fs::create_dir_all(&assets).unwrap();
    for name in [
        "page-9.png",
        "page-3-pic-2.jpg",
        "page-4-code-1.png",
        "attachment-7.json",
        "old-2.bin",
        "logo.png",
        "logo.gif",
        "My Photo.png",
        "notes.txt",
        "page-1.tiff",
        "page-2.gif",
    ] {
        std::fs::write(assets.join(name), b"old").unwrap();
    }
    let (registry, _) = pictures(&["page-1"]);
    Pipeline::new(config(None), registry).run_bytes(
        "scan.txt",
        b"x".to_vec(),
        None,
        &written(out.path()),
    );
    assert_eq!(
        names_in(&assets),
        [
            "My Photo.png",
            "logo.gif",
            "logo.png",
            "notes.txt",
            "old-2.bin",
            "page-1.png",
            "page-1.tiff"
        ]
    );
}

/// A reader that always panics, for a failed read.
struct Panicking;

impl Reader for Panicking {
    fn id(&self) -> &'static str {
        "panicking"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn formats(&self) -> &'static [FormatId] {
        &[FormatId::Text]
    }
    fn read(&self, _src: &Source, _opts: &ReadOptions, _budget: &Budget) -> ReadOutcome {
        panic!("boom in the reader");
    }
}

#[test]
fn a_failed_read_keeps_the_old_pictures() {
    let out = tempfile::tempdir().unwrap();
    chapters_pipeline(config(None)).run_bytes(
        "notes.txt",
        SMALL.to_vec(),
        None,
        &written(out.path()),
    );
    let assets = out.path().join("notes.txt.assets");
    let pictures = names_in(&assets);
    assert_eq!(pictures, ["pic-1.png"]);
    // The reader fails, and detection fails.
    let mut registry = Registry::new();
    registry.register(Box::new(Panicking));
    let res = Pipeline::new(config(None), registry).run_bytes(
        "notes.txt",
        b"text".to_vec(),
        None,
        &written(out.path()),
    );
    assert_eq!(res.completeness, Completeness::Failed);
    assert_eq!(names_in(&assets), pictures);
    let mut pipeline = chapters_pipeline(config(None));
    pipeline.set_detector(panicking_detector);
    let res = pipeline.run_bytes("notes.txt", SMALL.to_vec(), None, &written(out.path()));
    assert_eq!(res.completeness, Completeness::Failed);
    assert_eq!(names_in(&assets), pictures);
}

/// A text reader that records whether each read asked for page geometry.
struct GeometrySeen {
    seen: Arc<std::sync::Mutex<Vec<bool>>>,
}

impl Reader for GeometrySeen {
    fn id(&self) -> &'static str {
        "geometry-seen"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn formats(&self) -> &'static [FormatId] {
        &[FormatId::Text]
    }
    fn read(&self, src: &Source, opts: &ReadOptions, _budget: &Budget) -> ReadOutcome {
        self.seen.lock().unwrap().push(opts.geometry);
        let mut document = Document::new(source_info(src));
        if opts.geometry {
            // Marks the document of a geometry run, so a store of it would show later.
            document.body.push(Block::Paragraph {
                inlines: vec![Inline::Text {
                    text: "with geometry".into(),
                    emphasis: false,
                    strong: false,
                    strike: false,
                }],
                role: None,
                anchor: None,
            });
        }
        ReadOutcome {
            blobs: Vec::new(),
            document,
            completeness: Completeness::Complete,
        }
    }
}

#[test]
fn geometry_request_skips_cache() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut registry = Registry::new();
    registry.register(Box::new(GeometrySeen { seen: seen.clone() }));
    let pipeline = Pipeline::new(config(Some(dir.path())), registry);
    let geo = Request {
        geometry: true,
        ..request()
    };
    pipeline.run_bytes("a.txt", b"hello".to_vec(), None, &geo);
    pipeline.run_bytes("a.txt", b"hello".to_vec(), None, &geo);
    assert_eq!(*seen.lock().unwrap(), [true, true]);
    assert!(cache_files(dir.path()).is_empty());
    seen.lock().unwrap().clear();
    // A normal run stores the entry; a geometry run does not look it up.
    pipeline.run_bytes("a.txt", b"hello".to_vec(), None, &request());
    assert_eq!(cache_files(dir.path()).len(), 1);
    pipeline.run_bytes("a.txt", b"hello".to_vec(), None, &geo);
    assert_eq!(*seen.lock().unwrap(), [false, true]);
    // The geometry run did not store: a later normal run takes the entry of the first
    // normal run from the cache (no read) and it has no mark of the geometry run.
    let later = pipeline.run_bytes("a.txt", b"hello".to_vec(), None, &request());
    assert_eq!(*seen.lock().unwrap(), [false, true]);
    assert!(paragraphs(&later.document.unwrap()).is_empty());
    assert_eq!(cache_files(dir.path()).len(), 1);
}
