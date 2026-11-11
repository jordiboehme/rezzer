// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `scorecard`: checks assertion files against Rezzer's model JSON, and makes them.
//!
//! - `scorecard check [--names] [--set <name>] [--tsv <file>] [--regions-tsv <file>]
//!   <assertions-dir> <json-dir>`: checks `a/b.pdf.facts` on `a/b.pdf.json` and prints, per
//!   file and class, `passed / total`, then the totals per half. Without `--names` a file is
//!   `#n` and no text of a fact or of a file is printed: this is how a private set is run.
//!   With `--names` (public files only) each file is named and its failing facts are listed.
//!   `#n` is the place of the file's read among all reads of the set, in the report and in
//!   an error alike.
//!
//! Without `--names` an internal error of any command prints `internal error`, with `in file
//! #n` when a read of a set was being checked, and nothing else.
//!   `--tsv` and `--regions-tsv` add the lines of the set named by `--set` to the scorecard
//!   and regions tables.
//! - `scorecard trace <json> <anchor or text>`: the kind, certainty, evidence ids and bounds
//!   of the region at the anchor, or of the regions that hold the text. No other text.
//! - `scorecard gen tagged <json>`, `gen invoice <pdf> <json>`, `gen fields <json>` and
//!   `gen all <pdf> <json>`: the facts a file gives by its tags, its invoice XML and its
//!   filled fields, from its PDF and the model JSON of its read with tags.
//! - `scorecard strip-tags <in.pdf> <out.pdf>`: the PDF without its tags. Exit code 3 and no
//!   file when it has none.
//! - `scorecard half <name>`: the half (`dev` or `held`) of the file or folder `name`.
//! - `scorecard summary <title> <scorecard.tsv> <regions.tsv>`: the Markdown summary.
//!
//! Files given to this tool are data: they are read, never run.

use std::collections::HashSet;
use std::io::Write as _;
use std::path::Path;
use std::process::ExitCode;

use rezzer_model::Document;
use scorecard::facts::Fact;
use scorecard::generate::{fields::fields, invoice, tagged::tagged, writable};
use scorecard::report::CURRENT;
use scorecard::report::{REGION_HEAD, SCORE_HEAD, score_set, summary};
use scorecard::split::{half, key};
use scorecard::strip::{Stripped, strip_tags};
use scorecard::trace::trace;

const USAGE: &str = "usage: scorecard check [--names] [--set <name>] [--tsv <file>] \
[--regions-tsv <file>] <assertions-dir> <json-dir>
       scorecard trace <json> <anchor or text>
       scorecard gen tagged <json>
       scorecard gen invoice <pdf> <json>
       scorecard gen fields <json>
       scorecard gen all <pdf> <json>
       scorecard strip-tags <in.pdf> <out.pdf>
       scorecard half <name>
       scorecard summary <title> <scorecard.tsv> <regions.tsv>";

/// What a command ends with besides success: a wrong call, a failure, or a PDF with no tags.
enum Stop {
    Usage,
    Failed(String),
    NotTagged,
}

impl From<String> for Stop {
    fn from(why: String) -> Stop {
        Stop::Failed(why)
    }
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}

fn document(path: &str) -> Result<Document, String> {
    serde_json::from_str(&read(path)?).map_err(|e| format!("{path}: it is no model JSON: {e}"))
}

/// Adds `lines` to the table at `path`, with `head` first when the file is new or empty.
fn append(path: &str, head: &str, lines: &str) -> Result<(), String> {
    let new = std::fs::metadata(path).map_or(true, |m| m.len() == 0);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("{path}: {e}"))?;
    let head = if new {
        format!("{head}\n")
    } else {
        String::new()
    };
    file.write_all(format!("{head}{lines}").as_bytes())
        .map_err(|e| format!("{path}: {e}"))
}

/// From here on a panic prints one line that names no file and quotes no text: `internal
/// error`, with `in file #n` when a read of a set was being checked. A panic message of Rust
/// may quote the text it failed on, which must not reach the output of a private run. Every
/// command runs so, but `check --names`, which is for public files.
fn quiet_panics() {
    std::panic::set_hook(Box::new(|_| {
        match CURRENT.load(std::sync::atomic::Ordering::Relaxed) {
            0 => eprintln!("scorecard: internal error"),
            n => eprintln!("scorecard: internal error in file #{n}"),
        }
    }));
}

fn check(args: &[String]) -> Result<(), Stop> {
    let (mut names, mut set, mut tsv, mut regions) = (false, None, None, None);
    let mut dirs = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--names" => names = true,
            "--set" => set = Some(args.next().ok_or(Stop::Usage)?),
            "--tsv" => tsv = Some(args.next().ok_or(Stop::Usage)?),
            "--regions-tsv" => regions = Some(args.next().ok_or(Stop::Usage)?),
            flag if flag.starts_with("--") => return Err(Stop::Usage),
            _ => dirs.push(arg),
        }
    }
    let [facts, json] = dirs.as_slice() else {
        return Err(Stop::Usage);
    };
    if (tsv.is_some() || regions.is_some()) && set.is_none() {
        return Err(Stop::Failed("--tsv and --regions-tsv need --set".into()));
    }
    let score = score_set(Path::new(facts), Path::new(json), names)?;
    print!("{}", score.report(names));
    let set = set.map_or("", String::as_str);
    if let Some(path) = tsv {
        append(path, SCORE_HEAD, &score.score_rows(set))?;
    }
    if let Some(path) = regions {
        append(path, REGION_HEAD, &score.region_rows(set))?;
    }
    Ok(())
}

/// The facts of one generator that may be written (see [`writable`]) and that no generator
/// before it wrote for this file (`seen`), with a comment line that names the generator;
/// nothing for no facts.
fn section(title: &str, facts: &[Fact], seen: &mut HashSet<String>) -> String {
    let lines: Vec<String> = facts
        .iter()
        .filter(|f| writable(f))
        .map(Fact::to_string)
        .filter(|line| seen.insert(line.clone()))
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    format!("# {title}\n{}\n", lines.join("\n"))
}

fn invoice_facts(pdf: &str, doc: &Document) -> Result<Vec<Fact>, String> {
    let bytes = std::fs::read(pdf).map_err(|e| format!("{pdf}: {e}"))?;
    match invoice::invoice_xml(&bytes).map_err(|why| format!("{pdf}: {why}"))? {
        Some(xml) => invoice::invoice(&xml, doc).map_err(|why| format!("{pdf}: {why}")),
        None => Ok(Vec::new()),
    }
}

const FROM_TAGS: &str = "from the tags of the file";
const FROM_INVOICE: &str = "from the invoice XML the file carries";
const FROM_FIELDS: &str = "from the filled form fields of the file";

fn generate(args: &[String]) -> Result<(), Stop> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let seen = &mut HashSet::new();
    let out = match args.as_slice() {
        ["tagged", json] => section(FROM_TAGS, &tagged(&document(json)?), seen),
        ["fields", json] => section(FROM_FIELDS, &fields(&document(json)?), seen),
        ["invoice", pdf, json] => {
            section(FROM_INVOICE, &invoice_facts(pdf, &document(json)?)?, seen)
        }
        ["all", pdf, json] => {
            let doc = document(json)?;
            // A file whose invoice XML is not read still gives its other facts.
            let from_xml = invoice_facts(pdf, &doc).unwrap_or_else(|why| {
                eprintln!("scorecard: {why}");
                Vec::new()
            });
            [
                section(FROM_TAGS, &tagged(&doc), seen),
                section(FROM_INVOICE, &from_xml, seen),
                section(FROM_FIELDS, &fields(&doc), seen),
            ]
            .concat()
        }
        _ => return Err(Stop::Usage),
    };
    print!("{out}");
    Ok(())
}

fn run(args: &[String]) -> Result<(), Stop> {
    let (command, rest) = args.split_first().ok_or(Stop::Usage)?;
    let texts: Vec<&str> = rest.iter().map(String::as_str).collect();
    match (command.as_str(), texts.as_slice()) {
        ("check", _) => check(rest),
        ("gen", _) => generate(rest),
        ("trace", [json, what]) => {
            let lines = trace(&document(json)?, what);
            if lines.is_empty() {
                return Err(Stop::Failed("not found".into()));
            }
            println!("{}", lines.join("\n"));
            Ok(())
        }
        ("strip-tags", [from, to]) => {
            let bytes = std::fs::read(from).map_err(|e| format!("{from}: {e}"))?;
            match strip_tags(&bytes).map_err(|why| format!("{from}: {why}"))? {
                Stripped::NotTagged => Err(Stop::NotTagged),
                Stripped::Done(out) => {
                    std::fs::write(to, out).map_err(|e| format!("{to}: {e}"))?;
                    Ok(())
                }
            }
        }
        ("half", [name]) => {
            println!("{}", half(&key(Path::new(name))).name());
            Ok(())
        }
        ("summary", [title, score, regions]) => {
            print!("{}", summary(title, &read(score)?, &read(regions)?)?);
            Ok(())
        }
        _ => Err(Stop::Usage),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.iter().any(|a| a == "--names") {
        quiet_panics();
    }
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Stop::Usage) => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
        Err(Stop::Failed(why)) => {
            eprintln!("scorecard: {why}");
            ExitCode::FAILURE
        }
        Err(Stop::NotTagged) => ExitCode::from(3),
    }
}
