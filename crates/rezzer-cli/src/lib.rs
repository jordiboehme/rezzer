// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The `rezzer` command line: `read` (alias `rez`), `inspect`, `formats` and `mcp`.
//!
//! Exit codes: 0 complete, 2 partial, 1 failed or any error (usage errors too, so 2 always
//! means a partial read). Errors go to stderr as one line `rezzer: <message>`.

mod formats;
mod inspect;
mod read;

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use rezzer_model::Registry;
use rezzer_pipeline::{Config, OutputSection};

#[derive(Debug, Parser)]
#[command(
    name = "rezzer",
    version,
    about = "Turns files into knowledge for AI agents: Markdown with frontmatter, tables and diagrams."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Read a file and print it as Markdown.
    #[command(alias = "rez")]
    Read(ReadArgs),
    /// Show the detected format of a file, its size and hash, without reading it.
    Inspect {
        /// The file to inspect.
        path: PathBuf,
    },
    /// List the formats this build can read.
    Formats,
    /// Run the MCP server on stdio.
    Mcp,
}

#[derive(Debug, Args)]
struct ReadArgs {
    /// The file to read.
    path: PathBuf,
    /// Write <DIR>/<file name>.md (and <file name>.json with --json) instead of printing,
    /// for example budget.xlsx.md. Pictures go to <DIR>/<file name>.assets/; older files
    /// there that this read does not write are removed.
    #[arg(short = 'o', long = "output-dir", value_name = "DIR")]
    output_dir: Option<PathBuf>,
    /// Read the password of an encrypted file from the first line of stdin. Needs a file path.
    #[arg(long)]
    password_stdin: bool,
    /// Print the document model as JSON instead of Markdown.
    #[arg(long)]
    json: bool,
    /// Print only this page (1-based); the page count goes to stderr.
    #[arg(long, value_name = "N", conflicts_with_all = ["json", "output_dir"])]
    page: Option<usize>,
    /// Only these sections, comma-separated: overview, properties, content, logic,
    /// annotations, changes, embedded, not_read.
    #[arg(long, value_name = "LIST", value_delimiter = ',')]
    sections: Option<Vec<OutputSection>>,
    /// Do not use the result cache.
    #[arg(long)]
    no_cache: bool,
    /// Do not show formulas as LaTeX math in the Logic section.
    #[arg(long)]
    no_math: bool,
}

/// Runs the command line with the readers of this build.
pub fn main(registry: Registry) -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            return if e.use_stderr() {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            };
        }
    };
    // A panic outside the pipeline's contained stages goes through the normal panic hook,
    // then ends as one `rezzer: internal error` line and exit 1.
    match rezzer_limits::contain(std::panic::AssertUnwindSafe(|| run(cli, registry))) {
        Ok(Ok(code)) => code,
        Ok(Err(message)) => fail(&message),
        Err(panic) => fail(&format!("internal error: {panic}")),
    }
}

fn run(cli: Cli, registry: Registry) -> Result<ExitCode, String> {
    match cli.command {
        Command::Read(args) => {
            let mut config = load_config()?;
            if args.no_cache {
                config.cache = false;
            }
            read::run(config, registry, &args)
        }
        Command::Inspect { path } => inspect::run(load_config()?, registry, &path),
        Command::Formats => formats::run(&registry),
        Command::Mcp => mcp(load_config()?, registry),
    }
}

/// How long `rezzer mcp` waits at exit for blocking work that is still running.
const MCP_SHUTDOWN_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Serves MCP on stdio until the client closes stdin. Errors go to stderr; stdout carries
/// only the protocol.
fn mcp(config: Config, registry: Registry) -> Result<ExitCode, String> {
    let pipeline = std::sync::Arc::new(rezzer_pipeline::Pipeline::new(config, registry));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the MCP server: {e}"))?;
    let served = runtime.block_on(rezzer_mcp::serve_stdio(pipeline));
    // Reads still running were told to stop; give them a short time, then exit anyway.
    runtime.shutdown_timeout(MCP_SHUTDOWN_WAIT);
    served.map_err(|e| format!("MCP server: {e}"))?;
    Ok(ExitCode::SUCCESS)
}

fn load_config() -> Result<Config, String> {
    Config::load().map_err(|e| e.to_string())
}

/// Prints `rezzer: <message>` as one line on stderr and returns exit code 1.
fn fail(message: &str) -> ExitCode {
    let line = message.split_whitespace().collect::<Vec<_>>().join(" ");
    let _ = writeln!(std::io::stderr(), "rezzer: {line}");
    ExitCode::FAILURE
}

/// Writes to stdout. A closed pipe (`rezzer read x | head`) is not an error worth a message.
fn print(text: &str) -> Result<(), String> {
    let mut out = std::io::stdout().lock();
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(format!("cannot write to stdout: {e}")),
    }
}
