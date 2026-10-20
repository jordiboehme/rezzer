// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Hostile input through the whole pipeline (text, Markdown and CSV readers, no cache): the
//! run ends cleanly, inside its limits, and says what it could not do. The files are made in
//! memory or in a temp dir; nothing is executed.
#![cfg(all(feature = "fmt-text", feature = "fmt-markdown", feature = "fmt-csv"))]

use std::time::{Duration, Instant};

use rezzer_model::{Completeness, LossReason, Registry};
use rezzer_pipeline::{Config, OutputKind, Pipeline, Request, RunResult};

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
    let mut bytes = b"a,b\n".to_vec();
    bytes.extend(std::iter::repeat_n(b'x', 50 * 1024 * 1024));
    bytes.push(b'\n');
    let start = Instant::now();
    let result = p.run_bytes("huge.csv", bytes, None, &request());
    let took = start.elapsed();
    assert!(took < limit + SLACK, "took {took:?}");
    assert!(
        matches!(
            result.completeness,
            Completeness::Partial | Completeness::Complete
        ),
        "{result:?}"
    );
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
    }
}
