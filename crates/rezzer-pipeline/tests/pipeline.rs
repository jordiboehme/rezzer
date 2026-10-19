// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rezzer_limits::Budget;
use rezzer_model::{
    Block, Completeness, Document, FormatId, Inline, LossReason, ReadOptions, ReadOutcome, Reader,
    Registry, Source, source_info,
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
fn cache_hit_takes_name_and_path_from_the_current_source() {
    let dir = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let a = files.path().join("a.txt");
    let b = files.path().join("b.txt");
    std::fs::write(&a, "hello\n").unwrap();
    std::fs::write(&b, "hello\n").unwrap();
    let (registry, calls) = counting(false);
    let pipeline = Pipeline::new(config(Some(dir.path())), registry);
    pipeline.run_path(&a, &request());
    let out = pipeline.run_path(&b, &request());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let doc = out.document.unwrap();
    assert_eq!(doc.source.name, "b.txt");
    assert_eq!(doc.source.path.as_deref(), Some(b.to_str().unwrap()));
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
fn generated_at_is_not_part_of_the_cache_key_but_options_are() {
    let dir = tempfile::tempdir().unwrap();
    let (registry, calls) = counting(false);
    let pipeline = Pipeline::new(config(Some(dir.path())), registry);
    pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &request());
    let later = Request {
        generated_at: Some("2027-01-01T00:00:00Z".into()),
        ..Request::default()
    };
    let out = pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &later);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(out.markdown_pages[0].contains("2027-01-01T00:00:00Z"));
    let sections = Request {
        sections: Some(vec![OutputSection::Content]),
        ..request()
    };
    pipeline.run_bytes("x.txt", b"hello\n".to_vec(), None, &sections);
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
            "cache = false\ncache_dir = {:?}\n\n[limits]\npage_tokens = 1000\nmax_time = 5\n\n[enrich]\ndisabled = [\"language\"]\n",
            cache.to_str().unwrap()
        ),
    )
    .unwrap();
    let cfg = Config::load_from(&path).unwrap();
    assert!(!cfg.cache);
    assert_eq!(cfg.cache_dir, Some(cache));
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
