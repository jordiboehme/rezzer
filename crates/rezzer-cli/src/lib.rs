// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The `rezzer` command line: `read` (alias `rez`), `inspect`, `formats` and `mcp`.
//!
//! Exit codes: 0 complete, 2 partial, 1 failed or any error (usage errors too, so 2 always
//! means a partial read). Errors go to stderr as one line `rezzer: <message>`.

mod derez;
mod formats;
mod inspect;
mod read;

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap::{Args, CommandFactory, Parser, Subcommand};
use rezzer_model::Registry;
use rezzer_pipeline::{Config, OutputSection, TableStyle};

/// What `-V` and `--version` print: the version, the copyright with the license and the
/// source link, in the same three lines as Crystalline. clap puts the program name in front
/// of the first line itself. Read from Cargo.toml, so the version, license and repository
/// never drift from the manifest.
const VERSION_BLOCK: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "\nCopyright (C) 2026 Jordi Böhme - ",
    env!("CARGO_PKG_LICENSE"),
    "\n",
    env!("CARGO_PKG_REPOSITORY"),
);

#[derive(Debug, Parser)]
#[command(
    name = "rezzer",
    version = VERSION_BLOCK,
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
    /// Run the MCP server on stdio. A file sent inline as a data: URI may be up to
    /// limits.max_inline_bytes (32 MiB by default); give a larger file by path.
    Mcp,
    // A hidden command that runs programs on the grid of a read file.
    // Hidden on purpose (Jordi, 2026-10-27): it stays out of help, docs and release notes.
    #[command(hide = true)]
    Derez(DerezArgs),
}

#[derive(Debug, Args)]
struct DerezArgs {
    path: Option<PathBuf>,
    #[arg(long, hide = true)]
    seed: Option<u64>,
}

#[derive(Debug, Args)]
struct ReadArgs {
    /// The file to read.
    path: PathBuf,
    /// Write <DIR>/<file name>.md (and <file name>.json with --json) instead of printing,
    /// for example budget.xlsx.md. Some names get a short hash added. A document over the
    /// part size is written as <file name>.md plus parts in a folder next to it,
    /// <file name>.parts. Pictures go to a folder next to it, <file name>.assets. In these
    /// folder names every character other than A to Z, a to z, 0 to 9, '.', '_' and '-'
    /// becomes '_', and the name then gets a short hash: 'Q3 report.pdf' gives
    /// Q3_report.pdf-6f38fa03.parts. Older files that Rezzer wrote there and this read does
    /// not write are removed. Rezzer only replaces or removes files it wrote for the same
    /// source. Anything else in the way stops the read. This includes the output of another
    /// file with the same name and other content. It also includes a name that differs only
    /// in case, on a file system that ignores case (the macOS default).
    #[arg(short = 'o', long = "output-dir", value_name = "DIR")]
    output_dir: Option<PathBuf>,
    /// Largest part in tokens when writing with -o. A larger document is written as a main
    /// document plus parts in a folder next to it. 0 writes one file. Default 10000.
    #[arg(long, value_name = "TOKENS", requires = "output_dir", value_parser = part_size)]
    part_size: Option<usize>,
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
    /// How sheet tables are written: table (Markdown tables) or records (one list item per
    /// row with the column header of each value). Default from the config file, else table.
    /// It changes only the Markdown: --json prints the full model.
    #[arg(long, value_name = "table|records")]
    table_style: Option<TableStyle>,
    /// Only this sheet, by name or by position as #1, #2 and so on. Give it more than once
    /// for more sheets. It changes only the Markdown: --json prints all sheets.
    #[arg(long = "sheet", value_name = "SELECTOR")]
    sheets: Vec<String>,
}

/// A `--part-size` value: a whole number, 0 or at least 2500.
fn part_size(text: &str) -> Result<usize, String> {
    let n: usize = text
        .trim()
        .parse()
        .map_err(|_| format!("{text:?} is not a whole number"))?;
    rezzer_limits::check_part_size(n)?;
    Ok(n)
}

/// Runs the command line with the readers of this build.
pub fn main(registry: Registry) -> ExitCode {
    main_with(registry, |_| {})
}

/// Like [`main`], and `after_read` gets the path of a file once `rezzer read` has finished
/// with it, as the last thing the command does.
pub fn main_with(registry: Registry, after_read: fn(&std::path::Path)) -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let e = hide_suggestion(e);
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
    match rezzer_limits::contain(std::panic::AssertUnwindSafe(|| {
        run(cli, registry, after_read)
    })) {
        Ok(Ok(code)) => code,
        Ok(Err(message)) => fail(&message),
        Err(panic) => fail(&format!("internal error: {panic}")),
    }
}

/// `e` without `derez` among its suggested subcommands, since clap suggests hidden
/// subcommands too. With other suggestions left, they stay; with none, the error reads as
/// clap's own for a subcommand that resembles nothing. Any other error is returned unchanged.
fn hide_suggestion(e: clap::Error) -> clap::Error {
    if e.kind() != ErrorKind::InvalidSubcommand {
        return e;
    }
    let Some(ContextValue::Strings(suggested)) = e.get(ContextKind::SuggestedSubcommand) else {
        return e;
    };
    if !suggested.iter().any(|s| s == "derez") {
        return e;
    }
    let others: Vec<String> = suggested
        .iter()
        .filter(|s| *s != "derez")
        .cloned()
        .collect();
    let mut out = clap::Error::new(ErrorKind::InvalidSubcommand).with_cmd(&Cli::command());
    if others.is_empty() {
        // As clap builds it when nothing is similar: the subcommand and the usage only.
        for kind in [ContextKind::InvalidSubcommand, ContextKind::Usage] {
            if let Some(value) = e.get(kind) {
                out.insert(kind, value.clone());
            }
        }
    } else {
        for (kind, value) in e.context() {
            let value = if kind == ContextKind::SuggestedSubcommand {
                ContextValue::Strings(others.clone())
            } else {
                value.clone()
            };
            out.insert(kind, value);
        }
    }
    out
}

fn run(cli: Cli, registry: Registry, after_read: fn(&std::path::Path)) -> Result<ExitCode, String> {
    match cli.command {
        Command::Read(args) => {
            let mut config = load_config()?;
            if args.no_cache {
                config.cache = false;
            }
            let code = read::run(config, registry, &args)?;
            after_read(&args.path);
            Ok(code)
        }
        Command::Inspect { path } => inspect::run(load_config()?, registry, &path),
        Command::Formats => formats::run(&registry),
        Command::Mcp => mcp(load_config()?, registry),
        Command::Derez(args) => {
            derez::check(&args)?;
            derez::run(load_config()?, registry, &args)
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn suggested(e: &clap::Error) -> Vec<String> {
        match e.get(ContextKind::SuggestedSubcommand) {
            Some(ContextValue::Strings(s)) => s.clone(),
            _ => Vec::new(),
        }
    }

    /// clap itself suggests `derez`; only `hide_suggestion` takes it out.
    #[test]
    fn clap_suggests_derez_and_hide_suggestion_drops_it() {
        let e = Cli::try_parse_from(["rezzer", "eread"]).unwrap_err();
        let before = suggested(&e);
        assert!(before.contains(&"derez".to_string()), "{before:?}");
        assert!(before.contains(&"read".to_string()), "{before:?}");
        assert_eq!(suggested(&hide_suggestion(e)), ["read"]);

        let e = Cli::try_parse_from(["rezzer", "dere"]).unwrap_err();
        assert_eq!(suggested(&e), ["derez"]);
        let e = hide_suggestion(e);
        assert_eq!(e.kind(), ErrorKind::InvalidSubcommand);
        assert!(suggested(&e).is_empty());
        let zzzz = Cli::try_parse_from(["rezzer", "zzzz"]).unwrap_err();
        assert_eq!(
            e.render().to_string().replace("dere", "zzzz"),
            zzzz.render().to_string()
        );
    }

    #[test]
    fn other_errors_pass_unchanged() {
        let e = Cli::try_parse_from(["rezzer", "rea"]).unwrap_err();
        let text = e.render().to_string();
        assert_eq!(hide_suggestion(e).render().to_string(), text);
        let e = Cli::try_parse_from(["rezzer", "read"]).unwrap_err();
        let text = e.render().to_string();
        assert_eq!(hide_suggestion(e).render().to_string(), text);
    }
}
