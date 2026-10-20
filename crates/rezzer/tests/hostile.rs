// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Hostile input through the whole pipeline (text, Markdown and CSV readers, no cache): the
//! run ends cleanly, inside its limits, and says what it could not do. The files are made in
//! memory or in a temp dir; nothing is executed.
#![cfg(all(feature = "fmt-text", feature = "fmt-markdown", feature = "fmt-csv"))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::{Duration, Instant};

use rezzer_model::{Completeness, LossReason, Registry};
use rezzer_pipeline::{Config, OutputKind, Pipeline, Request, RunResult};

/// Counts the heap of the thread that allocates, so tests that run side by side do not
/// disturb a measurement. The pipeline runs on the calling thread.
struct Counting;

thread_local! {
    static CURRENT: Cell<isize> = const { Cell::new(0) };
    static PEAK: Cell<isize> = const { Cell::new(0) };
}

fn grow(n: usize) {
    let now = CURRENT.with(|c| {
        c.set(c.get() + n as isize);
        c.get()
    });
    PEAK.with(|p| p.set(p.get().max(now)));
}

fn shrink(n: usize) {
    CURRENT.with(|c| c.set(c.get() - n as isize));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        grow(l.size());
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        shrink(l.size());
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        shrink(l.size());
        grow(new);
        unsafe { System.realloc(p, l, new) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Runs `f` and returns its result with the peak heap it added on this thread, in bytes.
fn peak_heap<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let base = CURRENT.with(Cell::get);
    PEAK.with(|p| p.set(base));
    let out = f();
    let peak = PEAK.with(Cell::get);
    (out, usize::try_from(peak - base).unwrap_or(0))
}

/// Allowed overshoot of a time limit (CI machines are slower than a laptop).
const SLACK: Duration = Duration::from_secs(2);

fn pipeline_with(max_time: Option<Duration>) -> Pipeline {
    let mut registry = Registry::new();
    registry.register(Box::new(rezzer_read_text::TextReader));
    registry.register(Box::new(rezzer_read_markdown::MarkdownReader));
    registry.register(Box::new(rezzer_read_csv::CsvReader));
    let mut config = Config {
        cache: false,
        ..Config::default()
    };
    if let Some(t) = max_time {
        config.limits.max_time = t;
    }
    Pipeline::new(config, registry)
}

fn request() -> Request {
    Request {
        output: OutputKind::Json,
        generated_at: Some("2026-01-01T00:00:00Z".to_string()),
        ..Request::default()
    }
}

/// What the CLI and the server give by default: Markdown pages, no JSON copy of the model.
fn markdown_request() -> Request {
    Request {
        output: OutputKind::Markdown,
        ..request()
    }
}

fn has_loss(result: &RunResult, why: LossReason) -> bool {
    result
        .document
        .as_ref()
        .is_some_and(|d| d.losses.iter().any(|l| l.why == why))
}

/// A cheap deterministic byte stream (xorshift), so the "random" input is the same each run.
fn noise(len: usize) -> Vec<u8> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

#[test]
fn deep_markdown_nesting_is_partial() {
    let mut bytes = ">".repeat(100_000).into_bytes();
    bytes.extend_from_slice(b" deep\n");
    let p = pipeline_with(None);
    let result = p.run_bytes("deep.md", bytes, None, &request());
    assert_eq!(result.completeness, Completeness::Partial, "{result:?}");
    assert!(has_loss(&result, LossReason::Limit));
}

#[test]
fn huge_csv_row_respects_time_limit() {
    let limit = Duration::from_secs(1);
    let p = pipeline_with(Some(limit));
    let mut bytes = Vec::new();
    bytes.extend(std::iter::repeat_n(b'x', 50 * 1024 * 1024));
    bytes.push(b'\n');
    let start = Instant::now();
    let result = p.run_bytes("huge.csv", bytes, None, &request());
    let took = start.elapsed();
    assert!(took < limit + SLACK, "took {took:?}");
    // Either the limit was hit (Partial with a Limit loss) or the run finished in time
    // (Complete); the test machine decides which, both are clean.
    match result.completeness {
        Completeness::Partial => assert!(has_loss(&result, LossReason::Limit), "{result:?}"),
        Completeness::Complete => {}
        Completeness::Failed => panic!("{result:?}"),
    }
    eprintln!("huge_csv_row: {:?} in {took:?}", result.completeness);
}

#[test]
fn many_csv_rows_stop_at_the_time_limit() {
    use std::io::Write;
    // Short limit and a file that needs far more than that to analyze, so the test also
    // fails on a fast machine when the analysis ignores the limit. The file is written in
    // a stream to a temp dir and read with run_path (memory-mapped).
    let limit = Duration::from_millis(50);
    let p = pipeline_with(Some(limit));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rows.csv");
    {
        let mut out = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        let mut written = 0usize;
        let mut i = 0u32;
        while written < 100 * 1024 * 1024 {
            let line = format!("{i},v{i}\n");
            out.write_all(line.as_bytes()).unwrap();
            written += line.len();
            i += 1;
        }
        out.flush().unwrap();
    }
    let start = Instant::now();
    let result = p.run_path(&path, &request());
    let took = start.elapsed();
    assert!(took < limit + SLACK, "took {took:?}");
    assert_eq!(result.completeness, Completeness::Partial, "{result:?}");
    assert!(has_loss(&result, LossReason::Limit));
}

#[test]
fn file_over_size_limit_fails_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.txt");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(3 * 1024 * 1024 * 1024).unwrap();
    drop(file);
    let p = pipeline_with(None);
    let result = p.run_path(&path, &request());
    assert_eq!(result.completeness, Completeness::Failed);
    assert!(result.document.is_none());
    let message = result.message.expect("a message");
    assert!(
        message.contains("size") || message.contains("large"),
        "{message}"
    );
}

#[test]
fn binary_garbage_is_unknown_not_failed() {
    let p = pipeline_with(None);
    let result = p.run_bytes("blob.bin", noise(1024 * 1024), None, &request());
    assert_eq!(result.completeness, Completeness::Partial, "{result:?}");
    assert!(has_loss(&result, LossReason::Unsupported));
}

#[test]
fn invalid_utf8_in_markdown_decodes() {
    let p = pipeline_with(None);
    let bytes = b"# Title\n\nbad \xff\xfe\xc3\x28 bytes\n\n- item\n".to_vec();
    let result = p.run_bytes("bad.md", bytes, None, &request());
    assert_ne!(result.completeness, Completeness::Failed, "{result:?}");
    let md = result.markdown_pages.join("");
    assert!(md.contains("Title") && md.contains("item"), "{md}");
}

#[test]
fn nul_and_empty_inputs_do_not_panic() {
    let p = pipeline_with(None);
    for (name, bytes) in [
        ("empty.md", Vec::new()),
        ("nul.txt", vec![0u8; 4096]),
        ("nul.csv", b"a,b\n\0\0,\0\n".to_vec()),
        ("lone.md", b"\xef\xbb\xbf".to_vec()),
    ] {
        let result = p.run_bytes(name, bytes, None, &request());
        assert!(
            result.document.is_some() || result.message.is_some(),
            "{name}"
        );
        if name == "empty.md" || name == "lone.md" {
            assert_ne!(
                result.completeness,
                Completeness::Failed,
                "{name}: {result:?}"
            );
        }
    }
}

/// Writes `unit` repeated `times` times to `name` in a temp dir, in a stream.
fn streamed(dir: &std::path::Path, name: &str, unit: &[u8], times: usize) -> std::path::PathBuf {
    use std::io::Write;
    let path = dir.join(name);
    let mut out = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
    for _ in 0..times {
        out.write_all(unit).unwrap();
    }
    out.flush().unwrap();
    path
}

fn element_limit_loss(result: &RunResult) -> bool {
    result.document.as_ref().is_some_and(|d| {
        d.losses
            .iter()
            .any(|l| l.why == LossReason::Limit && l.what.starts_with("element limit reached"))
    })
}

#[test]
fn many_markdown_inlines_stop_at_the_element_limit() {
    // 1.2 million emphasized words are 2.4 million inlines, over the default of 2 million.
    let dir = tempfile::tempdir().unwrap();
    let path = streamed(dir.path(), "inlines.md", b"*a* ", 1_200_000);
    let result = pipeline_with(None).run_path(&path, &request());
    assert_eq!(
        result.completeness,
        Completeness::Partial,
        "{:?}",
        result.message
    );
    assert!(element_limit_loss(&result));
}

#[test]
fn many_text_paragraphs_stop_at_the_element_limit() {
    // 1.1 million paragraphs are 2.2 million elements.
    let dir = tempfile::tempdir().unwrap();
    let path = streamed(dir.path(), "paras.txt", b"a\n\n", 1_100_000);
    let result = pipeline_with(None).run_path(&path, &request());
    assert_eq!(
        result.completeness,
        Completeness::Partial,
        "{:?}",
        result.message
    );
    assert!(element_limit_loss(&result));
}

#[test]
fn huge_single_paragraph_is_bounded() {
    // 100 MB without a line break is one paragraph. The file is read from disk (memory
    // mapped), so the heap shows only the copies the pipeline makes.
    const SIZE: usize = 100 * 1000 * 1000;
    let dir = tempfile::tempdir().unwrap();
    let path = streamed(dir.path(), "one.txt", &[b'x'; 1000], SIZE / 1000);

    let (result, peak) = peak_heap(|| pipeline_with(None).run_path(&path, &markdown_request()));
    eprintln!("huge_single_paragraph: peak heap {peak} bytes for {SIZE} bytes of text");
    assert_ne!(result.completeness, Completeness::Failed, "{result:?}");
    assert!(peak < 4 * SIZE, "peak heap {peak} bytes");

    // With a text limit below the input the paragraph is cut and the loss says so.
    let mut config = Config {
        cache: false,
        ..Config::default()
    };
    config.limits.max_text_bytes = (SIZE / 2) as u64;
    let p = Pipeline::new(config, {
        let mut registry = Registry::new();
        registry.register(Box::new(rezzer_read_text::TextReader));
        registry
    });
    let (result, peak) = peak_heap(|| p.run_path(&path, &markdown_request()));
    eprintln!("huge_single_paragraph (limit): peak heap {peak} bytes");
    assert_eq!(result.completeness, Completeness::Partial, "{result:?}");
    assert!(has_loss(&result, LossReason::Limit));
    assert!(peak < 4 * SIZE, "peak heap {peak} bytes");
}
