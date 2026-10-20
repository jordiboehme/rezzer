// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Rezzer's MCP server: the tools `rez`, `inspect` and `formats` on top of the
//! [`Pipeline`], served on stdio by `rezzer mcp`.
//!
//! Every call that runs answers with one text block (the page, the JSON or the message) and
//! a structured object. Bad input is a result with status `failed` and `isError` set, never
//! a protocol error. The pipeline is synchronous and runs on tokio's blocking pool. Nothing
//! but the protocol goes to stdout.

pub mod tools;

use std::sync::Arc;

use rezzer_pipeline::Pipeline;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::service::ServerInitializeError;
use rmcp::{ErrorData, ServerHandler, ServiceExt, tool, tool_handler, tool_router};

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
}

#[tool_router]
impl RezzerServer {
    pub fn new(pipeline: Arc<Pipeline>) -> Self {
        RezzerServer {
            pipeline,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Read a file and return it as Markdown with frontmatter. Give exactly one of path (a local file), file (a data: URI) or source. Optional: page (starting at 1), sections (a list of section names) and format (markdown or json). The result says the status, the page count, the format, the SHA-256 and how many parts could not be read."
    )]
    async fn rez(
        &self,
        Parameters(params): Parameters<RezParams>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(self.blocking(move |p| tools::rez(p, params)).await)
    }

    #[tool(
        description = "Show the detected format of a local file, the detection evidence, its size, its SHA-256 and the reader that would read it, without reading it. Input: path."
    )]
    async fn inspect(
        &self,
        Parameters(params): Parameters<InspectParams>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(self.blocking(move |p| tools::inspect(p, params)).await)
    }

    #[tool(
        description = "List the formats this build can read, with the reader and its version for each. No input."
    )]
    async fn formats(&self) -> Result<CallToolResult, ErrorData> {
        Ok(self.blocking(tools::formats).await)
    }
}

impl RezzerServer {
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
                tools::simple_failed(format!("The call {reason}."))
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

/// Serves the tools on stdin and stdout until the client closes the connection. A client
/// that closes stdin before a session starts (only `server/discover`, or nothing at all) is
/// a normal end, not an error.
pub async fn serve_stdio(pipeline: Arc<Pipeline>) -> anyhow::Result<()> {
    let service = match RezzerServer::new(pipeline)
        .serve(rmcp::transport::stdio())
        .await
    {
        Ok(service) => service,
        Err(ServerInitializeError::ConnectionClosed(_)) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    service.waiting().await?;
    Ok(())
}
