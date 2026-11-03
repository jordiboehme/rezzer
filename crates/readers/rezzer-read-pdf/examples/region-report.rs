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
//!
//! `region-report counts [--per-file] [--check <file>] <set>...` reads every `.pdf` of each
//! set (a file or a folder, as for `dump`) with the regions of its pages and prints
//! `kind\t<set 1>\t<set 2>...`: how many regions of each kind the set has, one line per kind
//! in `Kind` order, then a line `total`. With `--per-file` it prints `file\tkind\tcount` for
//! each file and each kind it has instead. With `--check <file>` it compares the first set's
//! column with `<file>` (lines `kind\tcount`, as the first column prints) and exits 1 on a
//! difference, naming the old and the new line.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use layoutregions::Kind;
use rezzer_limits::{Budget, Limits};
use rezzer_model::{
    Bytes, Confidence, Detection, FormatId, ReadOptions, ReadOutcome, Reader, Source,
};
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

/// `bytes` as the source `name`, detected as PDF for certain.
fn source(name: &str, bytes: Vec<u8>) -> Source {
    let sha256: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Source {
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
    }
}

/// The budget of every read: the default limits with a time limit of one hour.
fn budget() -> Budget {
    Budget::new(Limits {
        max_time: Duration::from_secs(3600),
        ..Limits::default()
    })
}

/// The Markdown and the JSON of one read of `bytes`, named `name`.
fn read(name: &str, bytes: Vec<u8>) -> (String, String) {
    let out: ReadOutcome = PdfReader.read(&source(name, bytes), &ReadOptions::default(), &budget());
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

/// The `.pdf` files of `input` (a file, or a folder walked recursively) and the folder their
/// paths are relative to.
fn files_of(input: &str) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let path = Path::new(input);
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{input}: {e}"))?;
    if meta.is_dir() {
        let mut files = Vec::new();
        walk(path, &mut files).map_err(|e| format!("{input}: {e}"))?;
        Ok((path.to_path_buf(), files))
    } else {
        let root = path.parent().map(Path::to_path_buf).unwrap_or_default();
        Ok((root, vec![path.to_path_buf()]))
    }
}

/// The file name of `file`, as the name of its source.
fn name_of(file: &Path) -> String {
    file.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn dump(out: &Path, inputs: &[String]) -> Result<usize, String> {
    let mut count = 0;
    let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for input in inputs {
        let (root, files) = files_of(input)?;
        for file in files {
            let target = out_path(&root.to_string_lossy(), &file.to_string_lossy(), "md");
            if let Some(first) = seen.insert(target.clone(), file.display().to_string()) {
                return Err(format!(
                    "{first} and {} would both be dumped to {target}",
                    file.display()
                ));
            }
            let bytes = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let (markdown, json) = read(&name_of(&file), bytes);
            let (root, file) = (root.to_string_lossy(), file.to_string_lossy());
            let io = |e: std::io::Error| format!("{file}: {e}");
            write(out, &out_path(&root, &file, "md"), &markdown).map_err(io)?;
            write(out, &out_path(&root, &file, "json"), &json).map_err(io)?;
            count += 1;
        }
    }
    Ok(count)
}

/// How many regions of each kind the pages of `bytes` have.
fn regions_of(name: &str, bytes: Vec<u8>) -> BTreeMap<Kind, usize> {
    let (_, pages) = rezzer_read_pdf::read_with_regions(
        &source(name, bytes),
        &ReadOptions::default(),
        &budget(),
    );
    let mut counts = BTreeMap::new();
    for page in &pages {
        for (kind, n) in layoutregions::count_by_kind(&page.regions) {
            *counts.entry(kind).or_insert(0) += n;
        }
    }
    counts
}

/// The lines `kind\t<count>...` for each kind in `Kind` order, one count per column, then
/// `total`.
fn table(columns: &[BTreeMap<Kind, usize>]) -> Vec<String> {
    let mut lines: Vec<String> = Kind::ALL
        .iter()
        .map(|k| {
            let counts: String = columns
                .iter()
                .map(|c| format!("\t{}", c.get(k).copied().unwrap_or(0)))
                .collect();
            format!("{}{counts}", k.name())
        })
        .collect();
    let totals: String = columns
        .iter()
        .map(|c| format!("\t{}", c.values().sum::<usize>()))
        .collect();
    lines.push(format!("total{totals}"));
    lines
}

/// The lines of `old` and `new` that differ, each as its old and its new line.
fn moved(old: &[String], new: &[String]) -> Vec<String> {
    let n = old.len().max(new.len());
    (0..n)
        .filter_map(|i| {
            let (a, b) = (old.get(i), new.get(i));
            (a != b).then(|| {
                format!(
                    "old: {}\nnew: {}",
                    a.map_or("(none)", String::as_str),
                    b.map_or("(none)", String::as_str)
                )
            })
        })
        .collect()
}

/// The `counts` mode: see the module.
struct Counts {
    per_file: bool,
    check: Option<String>,
    sets: Vec<String>,
}

impl Counts {
    fn parse(args: &[String]) -> Option<Counts> {
        let mut counts = Counts {
            per_file: false,
            check: None,
            sets: Vec::new(),
        };
        let mut args = args.iter();
        while let Some(a) = args.next() {
            match a.as_str() {
                "--per-file" => counts.per_file = true,
                "--check" => counts.check = Some(args.next()?.clone()),
                _ => counts.sets.push(a.clone()),
            }
        }
        (!counts.sets.is_empty()).then_some(counts)
    }

    /// Prints the counts; `Ok(false)` when the check found a difference.
    fn run(&self) -> Result<bool, String> {
        let mut columns = Vec::new();
        for set in &self.sets {
            let (_, files) = files_of(set)?;
            let mut column: BTreeMap<Kind, usize> = BTreeMap::new();
            for file in files {
                let bytes = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
                let counts = regions_of(&name_of(&file), bytes);
                for (kind, n) in &counts {
                    if self.per_file {
                        println!("{}\t{}\t{n}", file.display(), kind.name());
                    }
                    *column.entry(*kind).or_insert(0) += n;
                }
            }
            columns.push(column);
        }
        if !self.per_file {
            for line in table(&columns) {
                println!("{line}");
            }
        }
        let Some(check) = &self.check else {
            return Ok(true);
        };
        let old: Vec<String> = std::fs::read_to_string(check)
            .map_err(|e| format!("{check}: {e}"))?
            .lines()
            .map(str::to_string)
            .collect();
        let new = table(&columns[..1]);
        let moved = moved(&old, &new);
        for m in &moved {
            eprintln!("{m}");
        }
        Ok(moved.is_empty())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|m| m == "counts") {
        let Some(counts) = Counts::parse(&args[1..]) else {
            eprintln!("usage: region-report counts [--per-file] [--check <file>] <set>...");
            std::process::exit(2);
        };
        match counts.run() {
            Ok(true) => return,
            Ok(false) => std::process::exit(1),
            Err(e) => {
                eprintln!("region-report: {e}");
                std::process::exit(1);
            }
        }
    }
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
            eprintln!(
                "usage: region-report dump <out-dir> <input>...\n       region-report counts \
                 [--per-file] [--check <file>] <set>..."
            );
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_print_every_kind_then_the_total() {
        let mut a = BTreeMap::new();
        a.insert(Kind::Table, 2);
        a.insert(Kind::HeaderBand, 1);
        let b = BTreeMap::new();
        let lines = table(&[a, b]);
        assert_eq!(lines.len(), Kind::ALL.len() + 1);
        assert_eq!(lines[0], "header_band\t1\t0");
        assert!(lines.contains(&"table\t2\t0".to_string()));
        assert_eq!(lines.last().unwrap(), "total\t3\t0");
    }

    #[test]
    fn a_moved_count_names_its_old_and_new_line() {
        let old = vec!["table\t2".to_string(), "total\t2".to_string()];
        let new = vec!["table\t3".to_string(), "total\t3".to_string()];
        assert_eq!(
            moved(&old, &new),
            [
                "old: table\t2\nnew: table\t3",
                "old: total\t2\nnew: total\t3"
            ]
        );
        assert!(moved(&old, &old).is_empty());
    }

    #[test]
    fn counts_options_parse() {
        let args: Vec<String> = ["--per-file", "a", "--check", "f.tsv", "b"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let c = Counts::parse(&args).unwrap();
        assert!(c.per_file);
        assert_eq!(c.check.as_deref(), Some("f.tsv"));
        assert_eq!(c.sets, ["a", "b"]);
        assert!(Counts::parse(&[]).is_none());
        assert!(Counts::parse(&["--check".to_string()]).is_none());
    }

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
