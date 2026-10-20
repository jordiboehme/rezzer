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
    /// Write <DIR>/<name>.md (and <name>.json with --json) instead of printing.
    #[arg(short = 'o', long = "output-dir", value_name = "DIR")]
    output_dir: Option<PathBuf>,
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
}

/// Runs the command line with the readers of this build.
pub fn main(registry: Registry) -> ExitCode {
    // Reader panics are contained and reported as one line; the default hook would print a
    // multi-line trace to stderr first.
    std::panic::set_hook(Box::new(|_| {}));
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
    // Panics outside the pipeline's own containment still end as one line and exit 1.
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
        Command::Mcp => Err("the MCP server is not available yet".to_string()),
    }
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
