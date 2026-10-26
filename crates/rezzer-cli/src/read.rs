// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `rezzer read <PATH>`.

use std::io::BufRead;
use std::process::ExitCode;

use rezzer_model::{Completeness, Registry};
use rezzer_pipeline::{Config, OutputKind, Pipeline, Request, RunResult};

use crate::{ReadArgs, print};

pub(crate) fn run(config: Config, registry: Registry, args: &ReadArgs) -> Result<ExitCode, String> {
    let pipeline = Pipeline::new(config, registry);
    let password = if args.password_stdin {
        if args.path.as_os_str() == "-" {
            return Err("--password-stdin needs a file path".to_string());
        }
        Some(read_password()?)
    } else {
        None
    };
    let req = Request {
        output: if args.json {
            OutputKind::Json
        } else {
            OutputKind::Markdown
        },
        sections: args.sections.clone(),
        math: !args.no_math,
        password,
        output_dir: args.output_dir.clone(),
        part_size: args.part_size,
        table_style: args.table_style,
        sheets: args.sheets.clone(),
        ..Request::default()
    };
    let result = pipeline.run_path(&args.path, &req);
    if result.document.is_none() {
        return Err(result
            .message
            .unwrap_or_else(|| "the file could not be read".to_string()));
    }
    // With -o the pipeline wrote the files, and nothing is printed.
    if args.output_dir.is_none() {
        print_output(&result, args.page)?;
    }
    if let Some(message) = &result.message {
        eprintln!("rezzer: {message}");
    }
    Ok(exit_code(result.completeness))
}

/// Prints the JSON, one page with its count on stderr, or the whole Markdown.
fn print_output(result: &RunResult, page: Option<usize>) -> Result<(), String> {
    if let Some(json) = &result.json {
        return print(&with_newline(json));
    }
    let Some(n) = page else {
        return print(&result.markdown_pages.concat());
    };
    let total = result.markdown_pages.len();
    let text = n
        .checked_sub(1)
        .and_then(|i| result.markdown_pages.get(i))
        .ok_or_else(|| {
            let pages = if total == 1 { "page" } else { "pages" };
            format!("page {n} does not exist: the output has {total} {pages}")
        })?;
    print(text)?;
    eprintln!("page {n} of {total}");
    Ok(())
}

/// The first line of stdin without its line ending. Never printed or logged.
fn read_password() -> Result<String, String> {
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| format!("cannot read the password from stdin: {e}"))?;
    Ok(line.trim_end_matches(['\n', '\r']).to_string())
}

fn exit_code(completeness: Completeness) -> ExitCode {
    match completeness {
        Completeness::Complete => ExitCode::SUCCESS,
        Completeness::Partial => ExitCode::from(2),
        Completeness::Failed => ExitCode::FAILURE,
    }
}

fn with_newline(text: &str) -> String {
    if text.ends_with('\n') {
        text.to_string()
    } else {
        format!("{text}\n")
    }
}
