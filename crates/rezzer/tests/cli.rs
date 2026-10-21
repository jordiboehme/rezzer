// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! End-to-end tests of the `rezzer` binary. Every command runs with HOME, XDG_CONFIG_HOME
//! and the cache dir inside a temp dir, so the user's real config and cache are never used.

use std::path::PathBuf;

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use rezzer_model::Document;
use tempfile::TempDir;

const AT: &str = "2026-10-19T08:30:00Z";

struct Env {
    root: TempDir,
}

impl Env {
    fn new() -> Env {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config/rezzer");
        std::fs::create_dir_all(&config).unwrap();
        let cache = root.path().join("cache");
        std::fs::write(
            config.join("config.toml"),
            format!("cache_dir = {:?}\n", cache.to_str().unwrap()),
        )
        .unwrap();
        std::fs::create_dir_all(root.path().join("home")).unwrap();
        Env { root }
    }

    fn cmd(&self) -> Command {
        let mut cmd = cargo_bin_cmd!("rezzer");
        cmd.env("HOME", self.root.path().join("home"))
            .env("XDG_CONFIG_HOME", self.root.path().join("config"))
            .env("REZZER_GENERATED_AT", AT)
            .current_dir(self.root.path());
        cmd
    }

    fn file(&self, name: &str, content: &[u8]) -> PathBuf {
        let path = self.root.path().join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    fn cache_entries(&self) -> usize {
        std::fs::read_dir(self.root.path().join("cache"))
            .map(|d| d.count())
            .unwrap_or(0)
    }
}

fn broken_csv() -> String {
    let mut csv = String::from("id,qty,price\n");
    for i in 1..=30 {
        match i {
            3 => csv.push_str("3,4\n"),
            4 => csv.push_str("4,7,8,9\n"),
            _ => csv.push_str(&format!("{i},{},{}\n", i * 3 % 7, i * 5 % 11)),
        }
    }
    csv
}

const MARKDOWN: &[u8] = b"# Notes\n\nSome text with *emphasis*.\n\n## Part\n\nMore.\n";

#[test]
fn read_markdown_prints_frontmatter() {
    let env = Env::new();
    let md = env.file("notes.md", MARKDOWN);
    env.cmd()
        .arg("read")
        .arg(&md)
        .assert()
        .code(0)
        .stdout(predicate::str::starts_with("---\ntype: document\n"))
        .stdout(predicate::str::contains(AT))
        .stdout(predicate::str::contains("Some text with"));
    assert_eq!(env.cache_entries(), 1, "the cache lives in the temp dir");
}

#[test]
fn rez_alias_works() {
    let env = Env::new();
    let md = env.file("notes.md", MARKDOWN);
    let read = env.cmd().arg("read").arg(&md).output().unwrap();
    let rez = env.cmd().arg("rez").arg(&md).output().unwrap();
    assert!(rez.status.success());
    assert_eq!(read.stdout, rez.stdout);
}

#[test]
fn missing_path_exits_1() {
    let env = Env::new();
    env.cmd()
        .args(["read", "does-not-exist.md"])
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::starts_with("rezzer: "))
        .stderr(predicate::str::contains("not found"))
        .stderr(predicate::function(|s: &str| s.lines().count() == 1));
}

#[test]
fn broken_csv_exits_2() {
    let env = Env::new();
    let csv = env.file("broken.csv", broken_csv().as_bytes());
    env.cmd()
        .arg("read")
        .arg(&csv)
        .assert()
        .code(2)
        .stdout(predicate::str::contains("completeness: partial"));
}

#[test]
fn unknown_format_exits_2_and_says_so() {
    let env = Env::new();
    let bytes: Vec<u8> = (0..512u32).map(|i| (i * 7 % 256) as u8).collect();
    let blob = env.file("blob.bin", &bytes);
    env.cmd()
        .arg("read")
        .arg(&blob)
        .assert()
        .code(2)
        .stdout(predicate::str::contains("Rezzer doesn't know this format."));
}

#[test]
fn json_output_is_model() {
    let env = Env::new();
    let md = env.file("notes.md", MARKDOWN);
    let out = env
        .cmd()
        .arg("read")
        .arg(&md)
        .arg("--json")
        .output()
        .unwrap();
    assert!(out.status.success());
    let doc: Document = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc.model_version, 1);
    assert_eq!(doc.source.name, "notes.md");
}

#[test]
fn page_prints_one_page_and_the_count() {
    let env = Env::new();
    let mut text = String::new();
    for i in 0..30 {
        text.push_str(&format!("## Section {i}\n\n"));
        text.push_str(&"Words in a paragraph that fills the page. ".repeat(20));
        text.push_str("\n\n");
    }
    let md = env.file("long.md", text.as_bytes());
    let config = env.root.path().join("config/rezzer/config.toml");
    let mut cfg = std::fs::read_to_string(&config).unwrap();
    cfg.push_str("\n[limits]\npage_tokens = 500\n");
    std::fs::write(&config, cfg).unwrap();

    let all = env.cmd().arg("read").arg(&md).output().unwrap();
    let first = env
        .cmd()
        .arg("read")
        .arg(&md)
        .args(["--page", "1"])
        .output()
        .unwrap();
    let second = env
        .cmd()
        .arg("read")
        .arg(&md)
        .args(["--page", "2"])
        .output()
        .unwrap();
    assert!(first.status.success());
    let stderr = String::from_utf8(first.stderr).unwrap();
    assert!(stderr.starts_with("page 1 of "), "{stderr}");
    let total: usize = stderr.trim()["page 1 of ".len()..].parse().unwrap();
    assert!(total > 1);
    assert!(first.stdout.starts_with(b"---\n"));
    assert!(!second.stdout.starts_with(b"---\n"));
    assert!(all.stdout.starts_with(&first.stdout));

    env.cmd()
        .arg("read")
        .arg(&md)
        .args(["--page", &(total + 1).to_string()])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(format!("has {total} pages")));
    env.cmd()
        .arg("read")
        .arg(&md)
        .args(["--page", "0"])
        .assert()
        .code(1);
}

#[test]
fn sections_limit_the_output() {
    let env = Env::new();
    let csv = env.file("broken.csv", broken_csv().as_bytes());
    let out = env
        .cmd()
        .arg("read")
        .arg(&csv)
        .args(["--sections", "not_read"])
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("## Not read"), "{text}");
    assert!(!text.contains("## Overview"), "{text}");
    env.cmd()
        .arg("read")
        .arg(&csv)
        .args(["--sections", "content,bogus"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("bogus"));
}

#[cfg(feature = "fmt-xlsx")]
#[test]
fn no_math_drops_the_math_column() {
    let env = Env::new();
    let book = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/office/tunnel_design_calculations.xlsx");
    let logic = |extra: &[&str]| {
        let out = env
            .cmd()
            .arg("read")
            .arg(&book)
            .args(["--sections", "logic"])
            .args(extra)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0));
        String::from_utf8(out.stdout).unwrap()
    };
    let with = logic(&[]);
    assert!(with.contains("| Value | Math |"), "{with}");
    assert!(with.contains("$`"), "{with}");
    // The second run renders the cached model: math is no part of the cache key.
    let without = logic(&["--no-math"]);
    assert_eq!(env.cache_entries(), 1);
    assert!(without.contains("### Structural\\_Lining"), "{without}");
    assert!(!without.contains("Math"), "{without}");
    assert!(!without.contains("$`"), "{without}");
}

#[test]
fn output_dir_writes_markdown_and_json() {
    let env = Env::new();
    let md = env.file("notes.md", MARKDOWN);
    let dir = env.root.path().join("out/nested");
    env.cmd()
        .arg("read")
        .arg(&md)
        .arg("-o")
        .arg(&dir)
        .arg("--json")
        .assert()
        .code(0)
        .stdout("");
    // Named after the whole file name, so notes.md and notes.csv never overwrite each other.
    let written = std::fs::read_to_string(dir.join("notes.md.md")).unwrap();
    assert!(written.starts_with("---\ntype: document\n"));
    let json = std::fs::read(dir.join("notes.md.json")).unwrap();
    serde_json::from_slice::<Document>(&json).unwrap();

    let csv = env.file("notes.csv", b"a,b\n1,2\n3,4\n");
    env.cmd()
        .arg("read")
        .arg(&csv)
        .arg("-o")
        .arg(&dir)
        .assert()
        .code(0);
    assert!(dir.join("notes.csv.md").exists());
    assert_eq!(
        std::fs::read_to_string(dir.join("notes.md.md")).unwrap(),
        written
    );
}

#[test]
fn resource_is_the_canonical_absolute_path() {
    let env = Env::new();
    env.file("notes.md", MARKDOWN);
    let root = std::fs::canonicalize(env.root.path()).unwrap();
    let expected = format!("resource: \"{}\"", root.join("notes.md").display());
    env.cmd()
        .args(["read", "./notes.md"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains(expected));
}

#[test]
fn no_cache_leaves_the_cache_empty() {
    let env = Env::new();
    let md = env.file("notes.md", MARKDOWN);
    env.cmd()
        .arg("read")
        .arg(&md)
        .arg("--no-cache")
        .assert()
        .code(0);
    assert_eq!(env.cache_entries(), 0);
}

#[test]
fn bad_config_is_one_line_error() {
    let env = Env::new();
    let config = env.root.path().join("config/rezzer/config.toml");
    std::fs::write(&config, "colour = \"red\"\n").unwrap();
    let md = env.file("notes.md", MARKDOWN);
    env.cmd()
        .arg("read")
        .arg(&md)
        .assert()
        .code(1)
        .stderr(predicate::str::starts_with("rezzer: "))
        .stderr(predicate::str::contains("colour"))
        .stderr(predicate::str::contains(config.to_str().unwrap()))
        .stderr(predicate::function(|s: &str| s.lines().count() == 1));
}

#[test]
fn inspect_shows_format_and_sha() {
    let env = Env::new();
    let csv = env.file("t.csv", b"a,b\n1,2\n3,4\n");
    env.cmd()
        .arg("inspect")
        .arg(&csv)
        .assert()
        .code(0)
        .stdout(predicate::str::contains("format: csv"))
        .stdout(predicate::str::contains("confidence: "))
        .stdout(predicate::str::contains("evidence: "))
        .stdout(predicate::str::contains("size: 12 bytes"))
        .stdout(predicate::str::contains(
            "sha256: b9485148546419a0f6a85e8d708c923557c15d7f3c7d078ef1fa7f7c0f57d5a5",
        ))
        .stdout(predicate::str::contains("reader: csv"));
    env.cmd()
        .args(["inspect", "nope.bin"])
        .assert()
        .code(1)
        .stderr(predicate::str::starts_with("rezzer: file not found: "));
    std::fs::create_dir_all(env.root.path().join("sub")).unwrap();
    env.cmd()
        .args(["inspect", "sub"])
        .assert()
        .code(1)
        .stderr(predicate::str::starts_with("rezzer: not a regular file: "));
}

#[test]
fn formats_lists_text_markdown_csv() {
    let env = Env::new();
    let out = env.cmd().arg("formats").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    let rows: Vec<Vec<&str>> = text
        .lines()
        .map(|l| l.split_whitespace().collect())
        .collect();
    assert_eq!(rows[0], ["format", "reader", "version"]);
    let version = env!("CARGO_PKG_VERSION");
    for (format, reader) in [
        ("text", "text"),
        ("markdown", "markdown"),
        ("csv", "csv"),
        ("tsv", "csv"),
    ] {
        assert!(
            rows.iter().any(|r| r[..] == [format, reader, version]),
            "{format}: {text}"
        );
    }
}

#[test]
fn version_and_help_exit_0() {
    let env = Env::new();
    env.cmd()
        .arg("--version")
        .assert()
        .code(0)
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
    env.cmd().arg("--help").assert().code(0);
}

#[test]
fn usage_errors_exit_1_not_2() {
    let env = Env::new();
    env.cmd().arg("frobnicate").assert().code(1);
    env.cmd().arg("read").assert().code(1);
}

#[test]
fn mcp_serves_on_stdio_and_writes_only_the_protocol() {
    let env = Env::new();
    let input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        "\n",
    );
    let output = env.cmd().arg("mcp").write_stdin(input).output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let replies: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("every stdout line is JSON-RPC"))
        .collect();
    assert_eq!(replies.len(), 2, "{stdout}");
    assert_eq!(replies[0]["result"]["serverInfo"]["name"], "rezzer");
    let tools = replies[1]["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 3, "{stdout}");
}

#[test]
fn mcp_ends_cleanly_when_stdin_closes_before_a_session() {
    let env = Env::new();
    env.cmd()
        .arg("mcp")
        .write_stdin("")
        .assert()
        .code(0)
        .stdout("");
}

fn corpus(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/office")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn rezzer_formats_lists_sheet_reader() {
    let env = Env::new();
    let out = env.cmd().arg("formats").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    let rows: Vec<Vec<&str>> = text
        .lines()
        .map(|l| l.split_whitespace().collect())
        .collect();
    let version = env!("CARGO_PKG_VERSION");
    for format in ["xlsx", "xlsm", "xlsb", "ods"] {
        assert!(
            rows.iter().any(|r| r[..] == [format, "sheet", version]),
            "{format}: {text}"
        );
    }
}

#[test]
fn xlsx_end_to_end_exit_0() {
    let env = Env::new();
    let book = env.file("book.xlsx", &corpus("one-variable.xlsx"));
    env.cmd()
        .arg("read")
        .arg(&book)
        .assert()
        .code(0)
        .stdout(predicate::str::contains("format: xlsx"))
        .stdout(predicate::str::contains("## Logic"));
}

#[test]
fn ods_end_to_end_exit_0() {
    let env = Env::new();
    let book = env.file("book.ods", &corpus("formula-across-sheets.ods"));
    env.cmd()
        .arg("read")
        .arg(&book)
        .assert()
        .code(0)
        .stdout(predicate::str::contains("format: ods"))
        .stdout(predicate::str::contains("## Logic"));
}

#[test]
fn xlsm_end_to_end_exit_0() {
    let env = Env::new();
    let book = env.file("macros.xlsx", &corpus("testNames.xlsm"));
    env.cmd()
        .arg("read")
        .arg(&book)
        .assert()
        .code(0)
        .stdout(predicate::str::contains("format: xlsm").and(predicate::str::contains("## Logic")));
}

#[test]
fn xlsx_named_csv_still_reads_as_xlsx() {
    let env = Env::new();
    let book = env.file("data.csv", &corpus("one-variable.xlsx"));
    env.cmd()
        .arg("read")
        .arg(&book)
        .assert()
        .code(0)
        .stdout(predicate::str::contains("format: xlsx"))
        .stdout(predicate::str::contains("## Logic"));
}

#[test]
fn password_stdin_sets_password() {
    let env = Env::new();
    let md = env.file("notes.md", MARKDOWN);
    // A run with a password is never cached, which shows the password reached the request.
    env.cmd()
        .arg("read")
        .arg(&md)
        .arg("--password-stdin")
        .write_stdin("secret\r\nignored\n")
        .assert()
        .code(0);
    assert_eq!(env.cache_entries(), 0);
    env.cmd().arg("read").arg(&md).assert().code(0);
    assert_eq!(env.cache_entries(), 1);
}

#[test]
fn password_stdin_with_dash_is_error() {
    let env = Env::new();
    env.cmd()
        .args(["read", "-", "--password-stdin"])
        .write_stdin("secret\n")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--password-stdin needs a file path",
        ));
}

#[cfg(feature = "fmt-pdf")]
fn pdf_corpus(name: &str) -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/pdf")
        .join(name)
}

/// The user password of `pdf/qpdf-user-password.pdf` (see the corpus MANIFEST).
#[cfg(feature = "fmt-pdf")]
const PDF_PASSWORD: &str = "view";

/// The text without the word "overview", in lower case: the password `view` must not be in
/// what is left.
#[cfg(feature = "fmt-pdf")]
fn without_overview(text: &str) -> String {
    text.to_lowercase().replace("overview", "")
}

#[cfg(feature = "fmt-pdf")]
#[test]
fn rezzer_formats_lists_pdf_reader() {
    let env = Env::new();
    let out = env.cmd().arg("formats").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        text.lines()
            .map(|l| l.split_whitespace().collect::<Vec<_>>())
            .any(|r| r[..] == ["pdf", "pdf", version]),
        "{text}"
    );
}

#[cfg(feature = "fmt-pdf")]
#[test]
fn pdf_end_to_end_exit_0() {
    let env = Env::new();
    // Named .txt: the format is found by content.
    let file = env.file(
        "form.txt",
        &std::fs::read(pdf_corpus("pdfjs-text-field-calc.pdf")).unwrap(),
    );
    env.cmd()
        .arg("read")
        .arg(&file)
        .assert()
        .code(0)
        .stdout(predicate::str::contains("format: pdf"))
        .stdout(predicate::str::contains("completeness: complete"))
        .stdout(predicate::str::contains("### Form"));
    assert_eq!(env.cache_entries(), 1);
}

#[cfg(feature = "fmt-pdf")]
#[test]
fn pdf_with_user_password_exits_1_without_password() {
    let env = Env::new();
    env.cmd()
        .arg("read")
        .arg(pdf_corpus("qpdf-user-password.pdf"))
        .assert()
        .code(1)
        .stdout(predicate::str::contains("completeness: failed"))
        .stdout(predicate::str::contains(
            "This PDF is protected with a password and is not read.",
        ));
    // A wrong password says so and fails too.
    env.cmd()
        .arg("read")
        .arg(pdf_corpus("qpdf-user-password.pdf"))
        .arg("--password-stdin")
        .write_stdin("wrong\n")
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "The given password does not open this PDF.",
        ));
}

#[cfg(feature = "fmt-pdf")]
#[test]
fn password_stdin_reads_protected_pdf() {
    let env = Env::new();
    env.cmd()
        .arg("read")
        .arg(pdf_corpus("qpdf-user-password.pdf"))
        .arg("--password-stdin")
        .write_stdin(format!("{PDF_PASSWORD}\n"))
        .assert()
        .code(0)
        .stdout(predicate::str::contains("completeness: complete"))
        .stdout(predicate::str::contains("encrypted: true"))
        .stdout(predicate::str::contains("pages: 30"));
}

#[cfg(feature = "fmt-pdf")]
#[test]
fn password_never_in_output_or_cache() {
    let env = Env::new();
    let file = pdf_corpus("qpdf-user-password.pdf");
    let dir = env.root.path().join("out");
    let mut seen = Vec::new();
    for extra in [
        &[][..],
        &["--json"][..],
        &["-o", dir.to_str().unwrap(), "--json"][..],
    ] {
        let out = env
            .cmd()
            .arg("read")
            .arg(&file)
            .arg("--password-stdin")
            .args(extra)
            .write_stdin(format!("{PDF_PASSWORD}\n"))
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "{extra:?}");
        seen.push(String::from_utf8(out.stdout).unwrap());
        seen.push(String::from_utf8(out.stderr).unwrap());
    }
    // The JSON on stdout is the model; it is complete and holds no password.
    let doc: Document = serde_json::from_str(&seen[2]).unwrap();
    assert!(!doc.body.is_empty());
    for entry in std::fs::read_dir(&dir).unwrap() {
        seen.push(std::fs::read_to_string(entry.unwrap().path()).unwrap());
    }
    for text in &seen {
        assert!(!without_overview(text).contains(PDF_PASSWORD), "{text}");
    }
    // A read with a password is never cached.
    assert_eq!(env.cache_entries(), 0);
}

#[cfg(feature = "fmt-pdf")]
#[test]
fn o_bundle_holds_page_png() {
    let env = Env::new();
    let dir = env.root.path().join("out");
    env.cmd()
        .arg("read")
        .arg(pdf_corpus("scan-truman-letter-nara.pdf"))
        .arg("-o")
        .arg(&dir)
        .assert()
        .code(0)
        .stdout("");
    let md = std::fs::read_to_string(dir.join("scan-truman-letter-nara.pdf.md")).unwrap();
    assert!(
        md.contains("scan-truman-letter-nara.pdf.assets/page-1.png"),
        "{md}"
    );
    let png = std::fs::read(dir.join("scan-truman-letter-nara.pdf.assets/page-1.png")).unwrap();
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    // Reads that extract assets bypass the cache.
    assert_eq!(env.cache_entries(), 0);
}

/// With every log level on, the password shows up neither on stderr nor in the MCP
/// replies, for the CLI and for the MCP `rez` tool.
#[cfg(feature = "fmt-pdf")]
#[test]
fn password_never_in_logs() {
    let env = Env::new();
    let file = pdf_corpus("qpdf-user-password.pdf");
    let out = env
        .cmd()
        .env("RUST_LOG", "trace")
        .env("RUST_BACKTRACE", "1")
        .arg("read")
        .arg(&file)
        .arg("--password-stdin")
        .write_stdin(format!("{PDF_PASSWORD}\n"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        !without_overview(&stderr).contains(PDF_PASSWORD),
        "{stderr}"
    );

    let call = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "rez",
            "arguments": { "path": file.to_str().unwrap(), "password": PDF_PASSWORD }
        }
    });
    let input = format!(
        "{}\n{}\n{}\n",
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        call
    );
    let out = env
        .cmd()
        .env("RUST_LOG", "trace")
        .arg("mcp")
        .write_stdin(input)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stdout.contains("encrypted: true"), "{stdout}");
    for text in [&stdout, &stderr] {
        assert!(!without_overview(text).contains(PDF_PASSWORD), "{text}");
    }
    assert_eq!(env.cache_entries(), 0);
}
