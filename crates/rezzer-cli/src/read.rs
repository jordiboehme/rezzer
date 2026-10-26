// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `rezzer read <PATH>`.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rezzer_model::{Completeness, Registry};
use rezzer_pipeline::{Config, OutputKind, Pipeline, Request};

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
    let markdown = result.markdown_pages.concat();
    if let Some(dir) = &args.output_dir {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let name = output_name(&args.path);
        write(&dir.join(format!("{name}.md")), &markdown)?;
        if let Some(json) = &result.json {
            write(&dir.join(format!("{name}.json")), &with_newline(json))?;
        }
    } else if let Some(json) = &result.json {
        print(&with_newline(json))?;
    } else if let Some(n) = args.page {
        let total = result.markdown_pages.len();
        let page = n
            .checked_sub(1)
            .and_then(|i| result.markdown_pages.get(i))
            .ok_or_else(|| {
                let pages = if total == 1 { "page" } else { "pages" };
                format!("page {n} does not exist: the output has {total} {pages}")
            })?;
        print(page)?;
        eprintln!("page {n} of {total}");
    } else {
        print(&markdown)?;
    }
    if let Some(message) = &result.message {
        eprintln!("rezzer: {message}");
    }
    Ok(exit_code(result.completeness))
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

/// The base of the output file names: the input's whole file name, so `a.csv` and `a.md`
/// give `a.csv.md` and `a.md.md` and never overwrite each other. Else `output`.
fn output_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "output".to_string())
}

fn write(path: &PathBuf, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn with_newline(text: &str) -> String {
    if text.ends_with('\n') {
        text.to_string()
    } else {
        format!("{text}\n")
    }
}
