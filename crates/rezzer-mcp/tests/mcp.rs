// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The MCP server through an in-process client over a duplex transport.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rezzer_limits::Budget;
use rezzer_model::{
    Completeness, Document, FormatId, ReadOptions, ReadOutcome, Reader, Registry, Source,
    source_info,
};
use rezzer_pipeline::{Config, Pipeline};
use rmcp::model::{CallToolRequest, CallToolRequestParams, CallToolResult, ClientRequest};
use rmcp::service::{PeerRequestOptions, RunningService};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};

fn registry() -> Registry {
    let mut registry = Registry::new();
    registry.register(Box::new(rezzer_read_text::TextReader));
    registry.register(Box::new(rezzer_read_markdown::MarkdownReader));
    registry.register(Box::new(rezzer_read_csv::CsvReader));
    registry
}

/// A config that never touches the user's cache.
fn config() -> Config {
    Config {
        cache: false,
        ..Config::default()
    }
}

async fn connect(config: Config) -> RunningService<RoleClient, ()> {
    connect_to(Pipeline::new(config, registry())).await
}

async fn connect_to(pipeline: Pipeline) -> RunningService<RoleClient, ()> {
    let pipeline = Arc::new(pipeline);
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        rezzer_mcp::serve(pipeline, server_io)
            .await
            .expect("the server runs");
    });
    ().serve(client_io).await.expect("the client connects")
}

async fn call(client: &RunningService<RoleClient, ()>, tool: &str, args: Value) -> CallToolResult {
    let params = CallToolRequestParams::new(tool.to_string())
        .with_arguments(args.as_object().expect("an object").clone());
    client
        .call_tool(params)
        .await
        .expect("a tool result, not a protocol error")
}

fn text(result: &CallToolResult) -> &str {
    assert_eq!(result.content.len(), 1, "one text block: {result:?}");
    &result.content[0].as_text().expect("a text block").text
}

fn structured(result: &CallToolResult) -> &Value {
    result
        .structured_content
        .as_ref()
        .expect("structured content")
}

fn write(dir: &Path, name: &str, content: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path.to_string_lossy().into_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_three_tools() {
    let client = connect(config()).await;
    let mut names: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    names.sort();
    assert_eq!(names, ["formats", "inspect", "rez"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn server_info_names_rezzer_and_says_what_it_does() {
    let client = connect(config()).await;
    let info = client.peer_info().expect("server info");
    let implementation = info.server_info.as_ref().expect("an implementation");
    assert_eq!(implementation.name, "rezzer");
    assert_eq!(implementation.version, env!("CARGO_PKG_VERSION"));
    let instructions = info.instructions.as_deref().expect("instructions");
    assert!(instructions.contains("rez"), "{instructions}");
    assert!(instructions.contains("local path"), "{instructions}");
}

#[tokio::test(flavor = "multi_thread")]
async fn rez_markdown_page_1() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "note.md", "# Title\n\nHello world.\n");
    let client = connect(config()).await;
    let result = call(&client, "rez", json!({ "path": path })).await;
    assert_ne!(result.is_error, Some(true));
    let page = text(&result);
    assert!(page.starts_with("---\n"), "{page}");
    assert!(page.contains("Hello world."), "{page}");
    let s = structured(&result);
    assert_eq!(s["status"], "complete", "{s}");
    assert_eq!(s["page"], 1);
    assert_eq!(s["pages"], 1);
    assert_eq!(s["format"], "markdown");
    assert_eq!(s["sha256"].as_str().unwrap().len(), 64);
    assert_eq!(s["losses"], 0);
    assert!(s.get("message").is_none(), "{s}");
}

#[tokio::test(flavor = "multi_thread")]
async fn rez_missing_path_is_failed_result() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.md");
    let client = connect(config()).await;
    let result = call(&client, "rez", json!({ "path": path })).await;
    assert_eq!(result.is_error, Some(true));
    let s = structured(&result);
    assert_eq!(s["status"], "failed", "{s}");
    let message = s["message"].as_str().expect("a message");
    assert!(message.contains("not found"), "{message}");
    assert_eq!(text(&result), message);
    assert_eq!(s["pages"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn rez_requires_exactly_one_input() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "a.txt", "text\n");
    let client = connect(config()).await;
    for args in [
        json!({}),
        json!({ "path": path, "file": "data:,x" }),
        json!({ "path": path, "source": "rezzer://x" }),
    ] {
        let result = call(&client, "rez", args.clone()).await;
        let s = structured(&result);
        assert_eq!(s["status"], "failed", "{args}: {s}");
        let message = s["message"].as_str().unwrap();
        assert!(
            message.contains("exactly one of path, file or source"),
            "{message}"
        );
        assert!(!message.ends_with('.'), "{message}");
    }
    // No arguments at all is the same rule.
    let result = client
        .call_tool(CallToolRequestParams::new("rez"))
        .await
        .unwrap();
    assert_eq!(structured(&result)["status"], "failed");
}

#[tokio::test(flavor = "multi_thread")]
async fn rez_data_uri_file() {
    let client = connect(config()).await;
    // "a,b\n1,2\n" in base64.
    let file = "data:text/csv;name=a.csv;base64,YSxiCjEsMgo=";
    let result = call(&client, "rez", json!({ "file": file })).await;
    let s = structured(&result);
    assert_eq!(s["status"], "complete", "{s}");
    assert_eq!(s["format"], "csv");
    assert!(text(&result).contains("a.csv"), "{}", text(&result));
}

#[tokio::test(flavor = "multi_thread")]
async fn rez_percent_encoded_data_uri() {
    let client = connect(config()).await;
    let file = "data:text/csv;name=b%20c.csv,a%2Cb%0A1%2C2%0A";
    let result = call(&client, "rez", json!({ "file": file })).await;
    let s = structured(&result);
    assert_eq!(s["status"], "complete", "{s}");
    assert_eq!(s["format"], "csv");
    assert!(text(&result).contains("b c.csv"), "{}", text(&result));
}

#[tokio::test(flavor = "multi_thread")]
async fn rez_bad_file_inputs_are_failed_results() {
    let mut small = config();
    small.limits.max_file_bytes = 4;
    let client = connect(small).await;
    for (file, expected) in [
        ("mcp-file:abc", "mcp-file references are not supported yet"),
        ("https://example.com/a.csv", "data: URI"),
        ("data:text/plain", "comma"),
        ("data:;base64,@@@", "base64"),
        ("data:;base64,YSxiCjEsMgo=", "too large"),
        ("data:,123456789", "too large"),
    ] {
        let result = call(&client, "rez", json!({ "file": file })).await;
        let s = structured(&result);
        assert_eq!(s["status"], "failed", "{file}: {s}");
        let message = s["message"].as_str().unwrap();
        assert!(message.contains(expected), "{file}: {message}");
        assert_eq!(text(&result), message);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn rez_source_is_not_available_yet() {
    let client = connect(config()).await;
    let result = call(
        &client,
        "rez",
        json!({ "source": "rezzer://sha256/x/a.csv" }),
    )
    .await;
    let s = structured(&result);
    assert_eq!(s["status"], "failed");
    assert_eq!(s["message"], "The source archive is not available yet");
}

#[tokio::test(flavor = "multi_thread")]
async fn rez_second_page() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "long.md", &long_markdown());
    let client = connect(config()).await;
    let first = call(&client, "rez", json!({ "path": path })).await;
    let pages = structured(&first)["pages"].as_u64().unwrap();
    assert!(pages >= 2, "{}", structured(&first));
    let second = call(&client, "rez", json!({ "path": path, "page": 2 })).await;
    let s = structured(&second);
    assert_eq!(s["status"], "complete", "{s}");
    assert_eq!(s["page"], 2);
    assert_eq!(s["pages"], pages);
    let page = text(&second);
    assert!(!page.starts_with("---"), "{}", &page[..80.min(page.len())]);
    assert!(page.contains("Words in a paragraph"));

    let beyond = call(&client, "rez", json!({ "path": path, "page": pages + 1 })).await;
    let s = structured(&beyond);
    assert_eq!(s["status"], "failed", "{s}");
    assert_eq!(s["pages"], pages);
    assert!(
        s["message"]
            .as_str()
            .unwrap()
            .contains(&format!("has {pages} pages")),
        "{s}"
    );
    let zero = call(&client, "rez", json!({ "path": path, "page": 0 })).await;
    assert_eq!(structured(&zero)["status"], "failed");
}

#[tokio::test(flavor = "multi_thread")]
async fn rez_sections_and_format() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "a.csv", "a,b\n1,2\n");
    let client = connect(config()).await;

    let unknown = call(
        &client,
        "rez",
        json!({ "path": path, "sections": ["nope"] }),
    )
    .await;
    let s = structured(&unknown);
    assert_eq!(s["status"], "failed");
    assert!(s["message"].as_str().unwrap().contains("nope"), "{s}");

    let bad = call(&client, "rez", json!({ "path": path, "format": "xml" })).await;
    let s = structured(&bad);
    assert_eq!(s["status"], "failed");
    assert!(s["message"].as_str().unwrap().contains("xml"), "{s}");

    let json_result = call(&client, "rez", json!({ "path": path, "format": "json" })).await;
    let s = structured(&json_result);
    assert_eq!(s["status"], "complete", "{s}");
    assert_eq!(s["pages"], 1);
    let model: Value = serde_json::from_str(text(&json_result)).expect("model JSON");
    assert_eq!(model["source"]["format"], "csv");

    let only = call(
        &client,
        "rez",
        json!({ "path": path, "sections": ["properties"] }),
    )
    .await;
    assert_eq!(structured(&only)["status"], "complete");
    assert!(!text(&only).contains("| 1 "), "{}", text(&only));
}

#[tokio::test(flavor = "multi_thread")]
async fn inspect_returns_sha256() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "hello.txt", "hello\n");
    let client = connect(config()).await;
    let result = call(&client, "inspect", json!({ "path": path })).await;
    let sha = "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03";
    let s = structured(&result);
    assert_eq!(s["sha256"], sha, "{s}");
    assert_eq!(s["format"], "text");
    assert_eq!(s["size"], 6);
    assert_eq!(s["reader"], "text");
    assert!(s["detection"]["confidence"].is_string(), "{s}");
    assert!(s["detection"]["evidence"].is_string(), "{s}");
    assert!(text(&result).contains(sha), "{}", text(&result));

    let missing = call(&client, "inspect", json!({ "path": dir.path().join("x") })).await;
    assert_eq!(missing.is_error, Some(true));
    assert_eq!(structured(&missing)["status"], "failed");
}

#[tokio::test(flavor = "multi_thread")]
async fn formats_lists_the_readers() {
    let client = connect(config()).await;
    let result = call(&client, "formats", json!({})).await;
    let formats = structured(&result)["formats"].as_array().unwrap().clone();
    assert!(
        formats
            .iter()
            .any(|f| f["format"] == "csv" && f["reader"] == "csv"),
        "{formats:?}"
    );
    assert!(text(&result).contains("csv"));
}

/// Markdown long enough for several pages.
fn long_markdown() -> String {
    let mut markdown = String::from("# Long\n\n");
    for i in 0..400 {
        markdown.push_str(&format!("## Part {i}\n\n"));
        markdown.push_str(&"Words in a paragraph that fills the page. ".repeat(20));
        markdown.push_str("\n\n");
    }
    markdown
}

#[tokio::test(flavor = "multi_thread")]
async fn paged_results_end_with_a_paging_line_after_the_page() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "long.md", &long_markdown());
    // The same pipeline directly, for the pages as the renderer wrote them. Pages after the
    // first hold no timestamp, so they are the same in both runs.
    let direct = Pipeline::new(config(), registry())
        .run_path(Path::new(&path), &rezzer_pipeline::Request::default());
    let total = direct.markdown_pages.len();
    assert!(total >= 3, "{total}");
    let client = connect(config()).await;

    for n in [2, total] {
        let result = call(&client, "rez", json!({ "path": path, "page": n })).await;
        let text = text(&result);
        let page = &direct.markdown_pages[n - 1];
        let rest = text
            .strip_prefix(page.as_str())
            .expect("the text starts with the page, unchanged");
        let line = if n < total {
            format!(
                "Page {n} of {total}. Call rez again with page: {} for the next page.",
                n + 1
            )
        } else {
            format!("Page {total} of {total}.")
        };
        let blank = if page.ends_with("\n\n") {
            ""
        } else if page.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        assert_eq!(rest, format!("{blank}{line}\n"), "page {n}");
        assert!(
            !page.contains("Page "),
            "the line is not in the page itself"
        );
    }

    let first = call(&client, "rez", json!({ "path": path })).await;
    assert!(
        text(&first).ends_with(&format!(
            "\n\nPage 1 of {total}. Call rez again with page: 2 for the next page.\n"
        )),
        "{}",
        &text(&first)[text(&first).len() - 120..]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn single_page_and_json_results_have_no_paging_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "a.csv", "a,b\n1,2\n");
    let client = connect(config()).await;
    let single = call(&client, "rez", json!({ "path": path })).await;
    assert!(!text(&single).contains("Page 1 of"), "{}", text(&single));
    let long = write(dir.path(), "long.md", &long_markdown());
    let json_result = call(&client, "rez", json!({ "path": long, "format": "json" })).await;
    assert!(!text(&json_result).contains("Page 1 of"));
    serde_json::from_str::<Value>(text(&json_result)).expect("the text is only the JSON");
}

#[tokio::test(flavor = "multi_thread")]
async fn rez_declares_an_output_schema() {
    let client = connect(config()).await;
    let tools = client.list_all_tools().await.unwrap();
    let rez = tools.iter().find(|t| t.name == "rez").unwrap();
    let schema = rez.output_schema.as_ref().expect("an output schema");
    let properties = schema["properties"].as_object().expect("properties");
    for key in [
        "status", "page", "pages", "format", "sha256", "losses", "message",
    ] {
        assert!(properties.contains_key(key), "{key}: {schema:?}");
    }
    assert_eq!(
        properties["status"]["enum"],
        json!(["complete", "partial", "failed"])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn lenient_page_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "long.md", &long_markdown());
    let client = connect(config()).await;
    for page in [json!(2), json!(2.0), json!("2"), json!(" 2 ")] {
        let result = call(&client, "rez", json!({ "path": path, "page": page })).await;
        let s = structured(&result);
        assert_eq!(s["status"], "complete", "{page}: {s}");
        assert_eq!(s["page"], 2, "{page}");
    }
    let huge = call(
        &client,
        "rez",
        json!({ "path": path, "page": 18446744073709551616.0 }),
    )
    .await;
    let s = structured(&huge);
    assert_eq!(s["status"], "failed", "{s}");
    assert!(
        s["message"].as_str().unwrap().contains("does not exist"),
        "{s}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_types_are_failed_results_with_structured_content() {
    let client = connect(config()).await;
    for (args, field) in [
        (json!({ "path": 5 }), "path"),
        (json!({ "file": true }), "file"),
        (json!({ "source": [] }), "source"),
        (json!({ "path": "a", "page": 1.5 }), "page"),
        (json!({ "path": "a", "page": "two" }), "page"),
        (json!({ "path": "a", "page": true }), "page"),
        (json!({ "path": "a", "sections": "content" }), "sections"),
        (json!({ "path": "a", "sections": [1] }), "sections"),
        (json!({ "path": "a", "format": 5 }), "format"),
    ] {
        let result = call(&client, "rez", args.clone()).await;
        assert_eq!(result.is_error, Some(true), "{args}");
        let s = structured(&result);
        assert_eq!(s["status"], "failed", "{args}: {s}");
        assert_eq!(s["pages"], 0, "{args}");
        let message = s["message"].as_str().unwrap();
        assert!(message.starts_with(field), "{args}: {message}");
        assert!(!message.ends_with('.'), "{args}: {message}");
        assert_eq!(text(&result), message);
    }
    let result = call(&client, "inspect", json!({ "path": 5 })).await;
    let s = structured(&result);
    assert_eq!(s["status"], "failed", "{s}");
    assert!(s["message"].as_str().unwrap().starts_with("path"), "{s}");
}

#[tokio::test(flavor = "multi_thread")]
async fn rez_input_schema_types_format_and_marks_the_file() {
    let mut cfg = config();
    cfg.limits.max_file_bytes = 1234;
    let client = connect(cfg).await;
    let tools = client.list_all_tools().await.unwrap();
    let rez = tools.iter().find(|t| t.name == "rez").unwrap();
    let properties = rez.input_schema["properties"].as_object().unwrap();
    let format = &properties["format"];
    assert_eq!(format["type"], "string", "{format}");
    assert_eq!(format["enum"], json!(["markdown", "json"]));
    assert_eq!(format["default"], "markdown");
    let required = rez.input_schema.get("required");
    assert!(
        required.is_none_or(|r| !r.as_array().unwrap().contains(&json!("format"))),
        "{required:?}"
    );
    assert_eq!(properties["page"]["type"], json!(["integer", "null"]));
    let file = &properties["file"];
    assert_eq!(file["format"], "uri", "{file}");
    assert_eq!(file["x-mcp-file"]["maxSize"], 1234, "{file}");
    assert!(properties["path"].get("x-mcp-file").is_none());
    let inspect = tools.iter().find(|t| t.name == "inspect").unwrap();
    assert_eq!(
        inspect.input_schema["properties"]["path"]["type"],
        json!(["string", "null"])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_base64_message_has_no_final_period() {
    let client = connect(config()).await;
    let result = call(&client, "rez", json!({ "file": "data:;base64,YW$j" })).await;
    let message = structured(&result)["message"].as_str().unwrap().to_string();
    assert!(message.contains("base64"), "{message}");
    assert!(
        !message.ends_with('.'),
        "error messages end without a period, as on the CLI: {message}"
    );
}

/// A text reader that runs until its budget stops it, and says whether a cancel did.
struct Slow {
    started: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
}

impl Reader for Slow {
    fn id(&self) -> &'static str {
        "slow"
    }
    fn version(&self) -> &'static str {
        "0.0.0"
    }
    fn formats(&self) -> &'static [FormatId] {
        &[FormatId::Text]
    }
    fn read(&self, src: &Source, _opts: &ReadOptions, budget: &Budget) -> ReadOutcome {
        self.started.store(true, Ordering::SeqCst);
        let end = Instant::now() + Duration::from_secs(60);
        while budget.check_time().is_ok() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(5));
        }
        self.cancelled
            .store(budget.is_cancelled(), Ordering::SeqCst);
        ReadOutcome {
            document: Document::new(source_info(src)),
            completeness: Completeness::Partial,
        }
    }
}

fn slow_pipeline() -> (Pipeline, Arc<AtomicBool>, Arc<AtomicBool>) {
    let started = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut registry = Registry::new();
    registry.register(Box::new(Slow {
        started: started.clone(),
        cancelled: cancelled.clone(),
    }));
    (Pipeline::new(config(), registry), started, cancelled)
}

async fn wait_for(flag: &AtomicBool, within: Duration) -> bool {
    let end = Instant::now() + within;
    while Instant::now() < end {
        if flag.load(Ordering::SeqCst) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

fn rez_request(args: Value) -> ClientRequest {
    ClientRequest::CallToolRequest(CallToolRequest::new(
        CallToolRequestParams::new("rez").with_arguments(args.as_object().unwrap().clone()),
    ))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_stops_a_long_run() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "a.txt", "some text\n");
    let (pipeline, started, cancelled) = slow_pipeline();
    let client = connect_to(pipeline).await;
    let handle = client
        .peer()
        .send_cancellable_request(
            rez_request(json!({ "path": path })),
            PeerRequestOptions::no_options(),
        )
        .await
        .unwrap();
    assert!(
        wait_for(&started, Duration::from_secs(10)).await,
        "the read started"
    );
    handle.cancel(None).await.unwrap();
    assert!(
        wait_for(&cancelled, Duration::from_secs(5)).await,
        "the cancel reached the budget"
    );
    // The server goes on serving.
    let result = call(&client, "formats", json!({})).await;
    assert!(text(&result).contains("slow"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_disconnect_stops_a_long_run() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "a.txt", "some text\n");
    let (pipeline, started, cancelled) = slow_pipeline();
    let client = connect_to(pipeline).await;
    let _handle = client
        .peer()
        .send_cancellable_request(
            rez_request(json!({ "path": path })),
            PeerRequestOptions::no_options(),
        )
        .await
        .unwrap();
    assert!(
        wait_for(&started, Duration::from_secs(10)).await,
        "the read started"
    );
    client.cancel().await.unwrap();
    // rmcp gives running calls up to 5 s to answer after the client is gone.
    assert!(
        wait_for(&cancelled, Duration::from_secs(10)).await,
        "the disconnect reached the budget"
    );
}
