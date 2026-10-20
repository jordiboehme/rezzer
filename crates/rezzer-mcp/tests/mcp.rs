// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The MCP server through an in-process client over a duplex transport.

use std::path::Path;
use std::sync::Arc;

use rezzer_model::Registry;
use rezzer_pipeline::{Config, Pipeline};
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
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
    let pipeline = Arc::new(Pipeline::new(config, registry()));
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let service = rezzer_mcp::RezzerServer::new(pipeline)
            .serve(server_io)
            .await
            .expect("the server starts");
        let _ = service.waiting().await;
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
    assert_eq!(s["message"], "The source archive is not available yet.");
}

#[tokio::test(flavor = "multi_thread")]
async fn rez_second_page() {
    let dir = tempfile::tempdir().unwrap();
    let mut markdown = String::from("# Long\n\n");
    for i in 0..400 {
        markdown.push_str(&format!("## Part {i}\n\n"));
        markdown.push_str(&"Words in a paragraph that fills the page. ".repeat(20));
        markdown.push_str("\n\n");
    }
    let path = write(dir.path(), "long.md", &markdown);
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
