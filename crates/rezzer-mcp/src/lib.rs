// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Rezzer's MCP server: the tools `rez`, `inspect` and `formats` on top of the
//! [`Pipeline`], served on stdio by `rezzer mcp`.
//!
//! Every call that runs answers with one text block (the page, the JSON or the message) and
//! a structured object. Bad input is a result with status `failed` and `isError` set, never
//! a protocol error. Only a message longer than the transport reads (see
//! [`transport::max_message_bytes`]) gets a JSON-RPC error, since it is not read. The
//! pipeline is synchronous and runs on tokio's blocking pool. A cancel from the client, or
//! the end of the connection, stops a running read at its next budget check. Nothing but the
//! protocol goes to stdout.

pub mod tools;
pub mod transport;

use std::sync::Arc;

use rezzer_pipeline::CancelToken;
use rezzer_pipeline::Pipeline;
use rmcp::handler::server::common::schema_for_type;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{
    CallToolResult, ContentBlock, Implementation, JsonObject, ServerCapabilities, ServerConfig,
};
use rmcp::service::{RequestContext, ServerInitializeError};
use rmcp::transport::IntoTransport;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::tools::{Answer, InspectParams, RezParams};

/// The server's name in the handshake.
pub const SERVER_NAME: &str = "rezzer";

/// What the server tells a connecting agent.
pub const INSTRUCTIONS: &str = "Rezzer reads files and returns their knowledge as Markdown \
with OKF frontmatter: structure, metadata, formulas, comments and what could not be read. \
Call rez with a local path. Long results come in pages.";

/// The MCP server. Cheap to clone; the pipeline is shared.
#[derive(Clone)]
pub struct RezzerServer {
    pipeline: Arc<Pipeline>,
    tool_router: ToolRouter<Self>,
    /// Set to true when the connection is over; running reads then stop.
    stop: Arc<watch::Sender<bool>>,
}

#[tool_router]
impl RezzerServer {
    pub fn new(pipeline: Arc<Pipeline>) -> Self {
        let mut tool_router = Self::tool_router();
        let max_inline_bytes = pipeline.config().limits.inline_bytes();
        if let Some(route) = tool_router.map.get_mut("rez") {
            route.attr.input_schema = Arc::new(rez_input_schema(max_inline_bytes));
        }
        if let Some(route) = tool_router.map.get_mut("inspect") {
            route.attr.input_schema = schema_for_type::<InspectParams>();
        }
        RezzerServer {
            pipeline,
            tool_router,
            stop: Arc::new(watch::channel(false).0),
        }
    }

    #[tool(
        output_schema = rmcp::handler::server::common::schema_for_output::<tools::RezOutput>(),
        description = "Read a file and return it as Markdown with frontmatter. Give exactly one of path (a local file), file (a data: URI) or source. Optional: page (starting at 1), sections (a list of section names), math (false leaves out the LaTeX form of formulas), format (markdown or json), table_style (table or records: one list item per row) and sheets (sheet names or positions such as #2). The result says the status, the page count, the format, the SHA-256 and how many parts could not be read. A result of several pages ends with a line that names the next page."
    )]
    async fn rez(
        &self,
        args: JsonObject,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let cancel = CancelToken::default();
        let watcher = self.cancel_on_end(&context, cancel.clone());
        let result = self.blocking(move |p| tools::rez(p, args, cancel)).await;
        watcher.abort();
        Ok(result)
    }

    #[tool(
        description = "Show the detected format of a local file, the detection evidence, its size, its SHA-256 and the reader that would read it, without reading it. Input: path."
    )]
    async fn inspect(&self, args: JsonObject) -> Result<CallToolResult, ErrorData> {
        Ok(self.blocking(move |p| tools::inspect(p, args)).await)
    }

    #[tool(
        description = "List the formats this build can read, with the reader and its version for each. No input."
    )]
    async fn formats(&self) -> Result<CallToolResult, ErrorData> {
        Ok(self.blocking(tools::formats).await)
    }
}

/// The input schema of `rez`: [`RezParams`] with the `file` property marked for the MCP
/// file extension draft (`x-mcp-file`), so a client that supports it can attach a file.
fn rez_input_schema(max_inline_bytes: u64) -> JsonObject {
    let mut schema = schema_for_type::<RezParams>().as_ref().clone();
    if let Some(Value::Object(file)) = schema.get_mut("properties").and_then(|p| p.get_mut("file"))
    {
        file.insert("x-mcp-file".into(), json!({ "maxSize": max_inline_bytes }));
    }
    schema
}

impl RezzerServer {
    /// Sets `cancel` when the client cancels this request or the connection ends. The
    /// returned task is aborted when the call is done.
    fn cancel_on_end(
        &self,
        context: &RequestContext<RoleServer>,
        cancel: CancelToken,
    ) -> tokio::task::JoinHandle<()> {
        let request = context.ct.clone();
        let mut stop = self.stop.subscribe();
        tokio::spawn(async move {
            tokio::select! {
                () = request.cancelled() => {}
                _ = stop.wait_for(|stopped| *stopped) => {}
            }
            cancel.cancel();
        })
    }

    /// Stops the reads that are still running: called when the connection is over.
    pub fn stop_running_reads(&self) {
        self.stop.send_replace(true);
    }

    /// Runs `work` on the blocking pool and turns its answer into a tool result. A panic
    /// that escapes the pipeline's own containment is a failed result too.
    async fn blocking(
        &self,
        work: impl FnOnce(&Pipeline) -> Answer + Send + 'static,
    ) -> CallToolResult {
        let pipeline = Arc::clone(&self.pipeline);
        let answer = tokio::task::spawn_blocking(move || work(&pipeline))
            .await
            .unwrap_or_else(|e| {
                let reason = if e.is_panic() {
                    "stopped with an internal error"
                } else {
                    "was cancelled"
                };
                tools::simple_failed(format!("The call {reason}"))
            });
        into_result(answer)
    }
}

fn into_result(answer: Answer) -> CallToolResult {
    let content = vec![ContentBlock::text(answer.text)];
    let mut result = if answer.failed {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    };
    result.structured_content = Some(answer.structured);
    result
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for RezzerServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(SERVER_NAME, env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

/// Serves the tools on stdin and stdout until the client closes the connection.
pub async fn serve_stdio(pipeline: Arc<Pipeline>) -> anyhow::Result<()> {
    let (read, write) = rmcp::transport::stdio();
    serve_io(pipeline, read, write).await
}

/// Serves the tools on one JSON-RPC message per line from `read`, answers to `write`. A
/// message may be as long as a `file` of the advertised `maxSize` needs; a longer one gets
/// an error answer.
pub async fn serve_io<R, W>(pipeline: Arc<Pipeline>, read: R, write: W) -> anyhow::Result<()>
where
    R: tokio::io::AsyncRead + Send + Unpin + 'static,
    W: tokio::io::AsyncWrite + Send + Unpin + 'static,
{
    let max = transport::max_message_bytes(pipeline.config().limits.inline_bytes());
    serve(pipeline, transport::LineTransport::new(read, write, max)).await
}

/// Serves the tools on `transport` until the client closes the connection. A client that
/// closes before a session starts (only `server/discover`, or nothing at all) is a normal
/// end, not an error. When the connection is over (after rmcp's grace time for answers
/// still being written), reads that are still running are told to stop.
pub async fn serve<T, E, A>(pipeline: Arc<Pipeline>, transport: T) -> anyhow::Result<()>
where
    T: IntoTransport<RoleServer, E, A>,
    E: std::error::Error + Send + Sync + 'static,
{
    let server = RezzerServer::new(pipeline);
    let stopper = server.clone();
    let service = match server.serve(transport).await {
        Ok(service) => service,
        Err(ServerInitializeError::ConnectionClosed(_)) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let quit = service.waiting().await;
    stopper.stop_running_reads();
    quit?;
    Ok(())
}
