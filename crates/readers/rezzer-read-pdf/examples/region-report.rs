// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reports on how the PDF reader reads a set of files.
//!
//! `region-report dump <out-dir> <input>...` reads every `.pdf` under the inputs (a file, or a
//! folder walked recursively, in the order of the paths) and writes `<out-dir>/<relative
//! path>.md` (rendered as the pipeline renders it) and `.json` (the `Document`). The time
//! limit is one hour, so no note about a time limit depends on the load of the machine and two
//! dumps of the same files are the same. The inputs are untrusted data and are only read.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rezzer_limits::{Budget, Limits};
use rezzer_model::{Bytes, Confidence, Detection, FormatId, ReadOptions, Reader, Source};
use rezzer_read_pdf::PdfReader;
use sha2::{Digest, Sha256};

/// The allocator of the Rezzer binary, so the reads run as they run in it.
#[global_allocator]
static ALLOCATOR: rezzer_limits::Allocator = rezzer_limits::Allocator;

/// The `generated.at` of every dump.
const GENERATED_AT: &str = "2026-01-01T00:00:00Z";

/// Where the dump of `file` goes: its path below `root`, with `.ext` added (`a/b.pdf.md`).
fn out_path(root: &str, file: &str, ext: &str) -> String {
    let relative = Path::new(file)
        .strip_prefix(root)
        .unwrap_or(Path::new(file));
    format!("{}.{ext}", relative.to_string_lossy())
}

/// Every `.pdf` file at `dir` and below, symbolic links left out, in the order of the paths.
fn walk(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            eprintln!("skipped, a symbolic link: {}", path.display());
        } else if kind.is_dir() {
            walk(&path, files)?;
        } else if is_pdf(&path) {
            files.push(path);
        }
    }
    Ok(())
}

fn is_pdf(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

/// The Markdown and the JSON of one read of `bytes`, named `name`.
fn read(name: &str, bytes: Vec<u8>) -> (String, String) {
    let sha256: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let source = Source {
        path: None,
        name: name.to_string(),
        size: bytes.len() as u64,
        bytes: Bytes::Owned(Arc::new(bytes)),
        sha256,
        format: FormatId::Pdf,
        detection: Detection {
            confidence: Confidence::Certain,
            evidence: "%PDF-".into(),
        },
        modified: None,
        created: None,
        downloaded_from: None,
        file_tags: Vec::new(),
    };
    let limits = Limits {
        max_time: Duration::from_secs(3600),
        ..Limits::default()
    };
    let out = PdfReader.read(&source, &ReadOptions::default(), &Budget::new(limits));
    let options = rezzer_render::RenderOptions {
        generated_at: GENERATED_AT.to_string(),
        rezzer_version: "dump".to_string(),
        ..rezzer_render::RenderOptions::default()
    };
    let markdown = rezzer_render::render(&out.document, out.completeness, &options);
    let json = serde_json::to_string_pretty(&out.document).expect("the model serializes");
    (markdown, json)
}

/// Writes `text` to `<out>/<relative>`, making the folders.
fn write(out: &Path, relative: &str, text: &str) -> std::io::Result<()> {
    let path = out.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, text)
}

fn dump(out: &Path, inputs: &[String]) -> Result<usize, String> {
    let mut count = 0;
    let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for input in inputs {
        let path = Path::new(input);
        let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{input}: {e}"))?;
        let (root, files) = if meta.is_dir() {
            let mut files = Vec::new();
            walk(path, &mut files).map_err(|e| format!("{input}: {e}"))?;
            (path.to_path_buf(), files)
        } else {
            let root = path.parent().map(Path::to_path_buf).unwrap_or_default();
            (root, vec![path.to_path_buf()])
        };
        for file in files {
            let target = out_path(&root.to_string_lossy(), &file.to_string_lossy(), "md");
            if let Some(first) = seen.insert(target.clone(), file.display().to_string()) {
                return Err(format!(
                    "{first} and {} would both be dumped to {target}",
                    file.display()
                ));
            }
            let bytes = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let name = file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let (markdown, json) = read(&name, bytes);
            let (root, file) = (root.to_string_lossy(), file.to_string_lossy());
            let io = |e: std::io::Error| format!("{file}: {e}");
            write(out, &out_path(&root, &file, "md"), &markdown).map_err(io)?;
            write(out, &out_path(&root, &file, "json"), &json).map_err(io)?;
            count += 1;
        }
    }
    Ok(count)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [mode, out, inputs @ ..] if mode == "dump" && !inputs.is_empty() => {
            match dump(Path::new(out), inputs) {
                Ok(n) => println!("{n} files dumped to {out}"),
                Err(e) => {
                    eprintln!("region-report: {e}");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("usage: region-report dump <out-dir> <input>...");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump_paths_mirror_inputs() {
        assert_eq!(
            out_path("tests/corpus/pdf", "tests/corpus/pdf/a/b.pdf", "md"),
            "a/b.pdf.md"
        );
        assert_eq!(
            out_path("tests/corpus/pdf", "tests/corpus/pdf/a/b.pdf", "json"),
            "a/b.pdf.json"
        );
    }
}
