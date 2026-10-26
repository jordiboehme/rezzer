// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Every file Rezzer writes for the corpus, checked against the OKF rules Crystalline 0.29.0
//! enforces, then imported into a throwaway Crystalline domain.
//!
//! [`write_corpus`] reads every corpus file into one output folder, plus copies of three
//! corpus files under names that break paths or permalinks (spaces, parentheses, `#`, `%`,
//! only non-ASCII, two names that sanitize alike, a data: URI name with `../`), an empty file
//! and an invented source with an NFD name. [`frontmatter_rules`] and [`bundle_rules`] check
//! what was written; `crystalline_imports_without_errors` runs the `crystalline` binary on it
//! when it is on `PATH`, with its config, state, data and index inside a temp folder, so no
//! real domain or index is ever touched. Rezzer itself never runs an external program; only
//! this test does. Corpus files are untrusted data and are only read by Rezzer, never run.
#![cfg(all(
    feature = "fmt-text",
    feature = "fmt-markdown",
    feature = "fmt-csv",
    feature = "fmt-xlsx",
    feature = "fmt-ods",
    feature = "fmt-pdf"
))]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::{GENERATED_AT, corpus_dir, corpus_files, pipeline};
use rezzer_pipeline::{Config, OutputKind, Request, RunResult};
use saphyr_parser::{Event, Parser};
use serde_json::Value;

/// The allocator of the Rezzer binary: the memory limits of the readers hold as shipped.
#[global_allocator]
static ALLOCATOR: rezzer_limits::Allocator = rezzer_limits::Allocator;

/// The NFD name of the invented source (ruling P5-W): `Ä` as `A` and a combining diaeresis.
const NFD_NAME: &str = "Bericht A\u{0308}nderung.md";

/// Copies of corpus files under the names of Review Focus 1, read from a path.
const NAMED_COPIES: [(&str, &str); 4] = [
    ("pdf/irs-f1040-2025.pdf", "Q3 report (final) #2 100%.pdf"),
    ("pdf/irs-f1040-2025.pdf", "a b.pdf"),
    ("pdf/irs-f1040-2025.pdf", "a_b.pdf"),
    ("office/XLOOKUP.xlsx", "日本.xlsx"),
];

/// A corpus file read from bytes under the name a data: URI gives (`;name=../../x.pdf`).
const DATA_URI_COPY: (&str, &str) = ("pdf/pdfbox-sf1199a-direct-deposit.pdf", "../../x.pdf");

/// Each Crystalline call ends after this long.
const CALL_TIMEOUT: Duration = Duration::from_secs(300);

fn default_part_size() -> usize {
    Config::default().limits.part_size
}

/// The part sizes the corpus is written at: the default and the smallest allowed.
fn part_sizes() -> [usize; 2] {
    [default_part_size(), rezzer_limits::MIN_PART_SIZE]
}

/// Reads every corpus file, the named copies, an empty file and the invented NFD source into
/// `dir` at `part_size`, with `generated.at` fixed. Returns the main documents written.
fn write_corpus(dir: &Path, part_size: usize) -> Vec<PathBuf> {
    let pipeline = pipeline();
    let req = Request {
        output: OutputKind::Markdown,
        output_dir: Some(dir.to_path_buf()),
        part_size: Some(part_size),
        generated_at: Some(GENERATED_AT.to_string()),
        ..Request::default()
    };
    let sources = tempfile::tempdir().unwrap();
    let mut paths = corpus_files();
    for (from, name) in NAMED_COPIES {
        let to = sources.path().join(name);
        std::fs::copy(corpus_dir().join(from), &to).unwrap();
        paths.push(to);
    }
    let empty = sources.path().join("empty.txt");
    std::fs::write(&empty, b"").unwrap();
    paths.push(empty);
    // Wikilinks whose targets Rezzer cannot know (ruling P5-AD): none may be a live link.
    let wiki = sources.path().join("wikilinks.md");
    std::fs::write(
        &wiki,
        "# Wikilinks\n\nSee [[project_plan]] and [[notes/my_plan.md]].\n\nAlso [[a|b]] and \
         [[graph-view]].\n\nIn code `[[kept]]` stays.\n",
    )
    .unwrap();
    paths.push(wiki);
    let nfd = sources.path().join(NFD_NAME);
    std::fs::write(&nfd, invented_report()).unwrap();
    paths.push(nfd);

    let mut written = Vec::new();
    let mut keep = |name: &str, res: RunResult| match res.main_file {
        Some(main) => written.push(main),
        None => panic!("{name}: nothing written: {:?}", res.message),
    };
    for path in &paths {
        keep(&path.display().to_string(), pipeline.run_path(path, &req));
    }
    let (from, name) = DATA_URI_COPY;
    let bytes = std::fs::read(corpus_dir().join(from)).unwrap();
    keep(name, pipeline.run_bytes(name, bytes, None, &req));
    written
}

/// An invented report in German, large enough to split into parts and, at the smallest part
/// size, to need contents parts. Nothing in it is real.
fn invented_report() -> String {
    let mut text = String::from(
        "# Bericht über die Änderung der Regeln\n\nEin erfundener Bericht für einen Test.\n",
    );
    for n in 1..=60 {
        text.push_str(&format!(
            "\n## Abschnitt {n}: Änderung der Regeln für den Bereich {n} und die Folgen für \
             alle Beteiligten\n"
        ));
        for _ in 0..8 {
            text.push('\n');
            for k in 0..14 {
                text.push_str(&format!(
                    "Satz {k} im Abschnitt {n} beschreibt eine erfundene Änderung, die niemand \
                     gemacht hat. "
                ));
            }
            text.push('\n');
        }
    }
    text
}

#[test]
fn frontmatter_rules_hold_on_corpus() {
    let mut problems = Vec::new();
    for part_size in part_sizes() {
        let out = tempfile::tempdir().unwrap();
        let mains = write_corpus(out.path(), part_size);
        assert!(mains.len() > 100, "{}", mains.len());
        for file in files_under(out.path()) {
            let shown = format!(
                "{} (part size {part_size})",
                file.strip_prefix(out.path()).unwrap().display()
            );
            problems.extend(
                file_name_rules(out.path(), &file)
                    .into_iter()
                    .map(|p| format!("{shown}: {p}")),
            );
            if file.extension().is_some_and(|e| e == "md") {
                problems.extend(
                    frontmatter_rules(&file)
                        .into_iter()
                        .chain(body_rules(out.path(), &file, part_size))
                        .map(|p| format!("{shown}: {p}")),
                );
            }
        }
        problems.extend(
            bundle_rules(out.path(), &mains)
                .into_iter()
                .map(|p| format!("part size {part_size}: {p}")),
        );
        // The rules above saw parts, and at the smallest size a list in contents parts.
        let names: BTreeSet<String> = files_under(out.path())
            .iter()
            .map(|f| f.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(names.iter().any(|n| n.ends_with(".md")), "{part_size}");
        if part_size == rezzer_limits::MIN_PART_SIZE {
            assert!(names.contains("contents-2.md"), "no contents parts");
        }
    }
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

/// The rules find what they look for: a file that breaks each of them.
#[test]
fn frontmatter_rules_catch_broken_files() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("bad.md");
    std::fs::write(
        &file,
        "\u{feff}---\ntype: document\ntitle: \"T\"\ntitle: \"U\"\ntags: [\"pdf\", \"A_b\"]\n\
         generated: x\nsource_date: 2026-02-30\ntemporal_confidence: high\npermalink: p\n---\n\n# T\n",
    )
    .unwrap();
    let problems = frontmatter_rules(&file).join("\n");
    for expected in ["byte order mark", "no frontmatter"] {
        assert!(problems.contains(expected), "{expected}: {problems}");
    }
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, text.trim_start_matches('\u{feff}')).unwrap();
    let problems = frontmatter_rules(&file).join("\n");
    for expected in [
        "repeated key `title`",
        "type is",
        "status is",
        "no description",
        "tags do not start with rezzer",
        "tag Scalar(\"A_b\")",
        "has permalink",
        "temporal_confidence Scalar(\"high\")",
        "source_date Scalar(\"2026-02-30\")",
        "generated is not a mapping",
    ] {
        assert!(problems.contains(expected), "{expected}: {problems}");
    }
    let body = body_rules(dir.path(), &file, 2_500).join("\n");
    assert!(body.contains("1 non-blank body lines"), "{body}");
    assert!(is_part_name("pages-1-2.md") && !is_part_name("-a.md") && !is_part_name("A.md"));
    assert!(is_date("2024-02-29") && !is_date("2023-02-29") && !is_date("2026-13-01"));
    assert_eq!(lines_outside_fences("a\n````\nb\n```\n````\nc\n"), 2);
}

// ---------------------------------------------------------------------------------------
// The rules

/// A YAML value as far as the rules need it.
#[derive(Debug, Clone, PartialEq)]
enum Yaml {
    Scalar(String),
    Seq(Vec<Yaml>),
    Map(Vec<(String, Yaml)>),
}

impl Yaml {
    fn get(&self, key: &str) -> Option<&Yaml> {
        match self {
            Yaml::Map(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn scalar(&self) -> Option<&str> {
        match self {
            Yaml::Scalar(s) => Some(s),
            _ => None,
        }
    }
}

/// The frontmatter text (between the `---` lines) and the body after the closing line.
fn split_frontmatter(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix("---\n")?;
    if let Some(body) = rest.strip_prefix("---\n") {
        return Some(("", body));
    }
    let end = rest.find("\n---\n")?;
    Some((&rest[..end + 1], &rest[end + 5..]))
}

/// Parses `yaml` into one document; repeated keys of any mapping are problems.
fn parse_yaml(yaml: &str, problems: &mut Vec<String>) -> Option<Yaml> {
    enum Open {
        Seq(Vec<Yaml>),
        Map(Vec<(String, Yaml)>, Option<String>),
    }
    fn push(stack: &mut [Open], top: &mut Option<Yaml>, value: Yaml, problems: &mut Vec<String>) {
        match stack.last_mut() {
            None => *top = Some(value),
            Some(Open::Seq(items)) => items.push(value),
            Some(Open::Map(entries, key)) => match key.take() {
                None => match value {
                    Yaml::Scalar(k) => {
                        if entries.iter().any(|(e, _)| *e == k) {
                            problems.push(format!("repeated key `{k}`"));
                        }
                        *key = Some(k);
                    }
                    other => problems.push(format!("a key that is not a scalar: {other:?}")),
                },
                Some(k) => entries.push((k, value)),
            },
        }
    }
    let mut stack: Vec<Open> = Vec::new();
    let mut top = None;
    for item in Parser::new_from_str(yaml) {
        let event = match item {
            Ok((event, _)) => event,
            Err(e) => {
                problems.push(format!("frontmatter is not YAML: {e}"));
                return None;
            }
        };
        match event {
            Event::Scalar(value, ..) => push(
                &mut stack,
                &mut top,
                Yaml::Scalar(value.into_owned()),
                problems,
            ),
            Event::SequenceStart(..) => stack.push(Open::Seq(Vec::new())),
            Event::MappingStart(..) => stack.push(Open::Map(Vec::new(), None)),
            Event::SequenceEnd | Event::MappingEnd => {
                let value = match stack.pop() {
                    Some(Open::Seq(items)) => Yaml::Seq(items),
                    Some(Open::Map(entries, _)) => Yaml::Map(entries),
                    None => return None,
                };
                push(&mut stack, &mut top, value, problems);
            }
            Event::Alias(_) => problems.push("an alias in the frontmatter".to_string()),
            _ => {}
        }
    }
    top
}

/// OKF and Crystalline rules on the frontmatter of `file`, as problems in words.
fn frontmatter_rules(file: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    let bytes = std::fs::read(file).unwrap();
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        problems.push("starts with a byte order mark".to_string());
    }
    if bytes.contains(&0) {
        problems.push("holds a NUL byte".to_string());
    }
    let Ok(text) = String::from_utf8(bytes) else {
        problems.push("not UTF-8".to_string());
        return problems;
    };
    let Some((yaml, _)) = split_frontmatter(&text) else {
        problems.push("no frontmatter between `---` lines".to_string());
        return problems;
    };
    let fm = match parse_yaml(yaml, &mut problems) {
        Some(fm @ Yaml::Map(_)) => fm,
        other => {
            problems.push(format!("frontmatter is not a mapping: {other:?}"));
            return problems;
        }
    };
    let scalar = |key: &str| fm.get(key).and_then(Yaml::scalar);
    if scalar("type") != Some("source") {
        problems.push(format!("type is {:?}, not source", fm.get("type")));
    }
    if scalar("status") != Some("stable") {
        problems.push(format!("status is {:?}, not stable", fm.get("status")));
    }
    for key in ["title", "description"] {
        if scalar(key).is_none_or(|v| v.trim().is_empty()) {
            problems.push(format!("no {key}"));
        }
    }
    match fm.get("tags") {
        Some(Yaml::Seq(tags)) => {
            if tags.first().and_then(Yaml::scalar) != Some("rezzer") {
                problems.push(format!("tags do not start with rezzer: {tags:?}"));
            }
            for tag in tags {
                if !tag.scalar().is_some_and(is_tag) {
                    problems.push(format!("tag {tag:?} is not [a-z0-9]+(-[a-z0-9]+)*"));
                }
            }
        }
        other => problems.push(format!("tags are not a list: {other:?}")),
    }
    for key in ["permalink", "recorded_at", "verified"] {
        if fm.get(key).is_some() {
            problems.push(format!("has {key}: the consumer fills it"));
        }
    }
    if let Some(confidence) = fm.get("temporal_confidence") {
        if fm.get("source_date").is_none() {
            problems.push("temporal_confidence without source_date".to_string());
        }
        if !matches!(confidence.scalar(), Some("explicit" | "inferred")) {
            problems.push(format!(
                "temporal_confidence {confidence:?} is not explicit or inferred"
            ));
        }
    }
    if let Some(date) = fm.get("source_date")
        && !date.scalar().is_some_and(is_date)
    {
        problems.push(format!(
            "source_date {date:?} is not a real YYYY-MM-DD date"
        ));
    }
    if !matches!(fm.get("generated"), Some(g @ Yaml::Map(_)) if g.get("by").is_some()) {
        problems.push(format!(
            "generated is not a mapping with by: {:?}",
            fm.get("generated")
        ));
    }
    problems
}

/// `[a-z0-9]+(-[a-z0-9]+)*`.
fn is_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.split('-').all(|w| {
            !w.is_empty()
                && w.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// `YYYY-MM-DD` and a day that exists.
fn is_date(text: &str) -> bool {
    let b = text.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let number = |r: std::ops::Range<usize>| -> Option<u32> {
        if b[r.clone()].iter().all(u8::is_ascii_digit) {
            text[r].parse().ok()
        } else {
            None
        }
    };
    let (Some(year), Some(month), Some(day)) = (number(0..4), number(5..7), number(8..10)) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

/// Rules on the body of `file`: Q002's size (decision 12) and Q001's three lines.
fn body_rules(root: &Path, file: &Path, part_size: usize) -> Vec<String> {
    let mut problems = Vec::new();
    let Ok(text) = std::fs::read_to_string(file) else {
        return problems;
    };
    let Some((_, body)) = split_frontmatter(&text) else {
        return problems;
    };
    let tokens = body_tokens(root, file, body);
    if tokens > part_size {
        problems.push(format!("{tokens} body tokens, over {part_size}"));
    }
    let lines = lines_outside_fences(body);
    if lines < 3 {
        problems.push(format!(
            "{lines} non-blank body lines outside fences, under 3"
        ));
    }
    problems
}

/// Q002's count: body characters divided by 4, each asset link counted at the longer of its
/// written length and the one Crystalline 0.29.0 rewrites it to on import
/// (`assets/<note path without .md>/<file name>`). The note path is taken below `root`, so
/// the count holds when `root` is the import root. The Crystalline test imports the folder
/// above (`out/size-<n>/`), so its rewritten links are longer by `size-<n>/`; there the
/// verify after the import is the real Q002 check.
fn body_tokens(root: &Path, file: &Path, body: &str) -> usize {
    let note = file.strip_prefix(root).unwrap().with_extension("");
    let note = note.to_str().unwrap();
    let mut chars = body.chars().count();
    for (_, dest) in body_links(body) {
        let path = dest.split('#').next().unwrap();
        if path.ends_with(".md") || has_scheme(path) || path.is_empty() {
            continue;
        }
        let name = path.rsplit('/').next().unwrap();
        let rewritten = "assets/".len() + note.chars().count() + 1 + name.chars().count();
        chars += rewritten.saturating_sub(dest.chars().count());
    }
    chars / 4
}

/// Crystalline's Q001 count: non-blank body lines outside fenced code, the fence lines
/// counted as inside.
fn lines_outside_fences(body: &str) -> usize {
    let mut fence: Option<(char, usize)> = None;
    let mut count = 0;
    for line in body.lines() {
        let trimmed = line.trim_start();
        let run = |c: char| trimmed.chars().take_while(|&x| x == c).count();
        match fence {
            Some((c, len)) => {
                if run(c) >= len && trimmed.trim_end().chars().all(|x| x == c) {
                    fence = None;
                }
            }
            None => {
                if let Some(c) = ['`', '~'].into_iter().find(|&c| run(c) >= 3) {
                    fence = Some((c, run(c)));
                } else if !trimmed.is_empty() {
                    count += 1;
                }
            }
        }
    }
    count
}

/// Rules on the name of every written file: no reserved name, and part file names.
fn file_name_rules(root: &Path, file: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    let name = file.file_name().unwrap().to_str().unwrap();
    if name == "index.md" || name == "log.md" {
        problems.push("a name Crystalline reserves".to_string());
    }
    if name.starts_with('.') {
        problems.push("a hidden name".to_string());
    }
    let parent = file.parent().unwrap();
    let in_parts = parent != root
        && parent
            .file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with(".parts"));
    if in_parts && !is_part_name(name) {
        problems.push("a part name outside ^[a-z0-9][a-z0-9-]*\\.md$".to_string());
    }
    problems
}

fn is_part_name(name: &str) -> bool {
    name.strip_suffix(".md").is_some_and(|stem| {
        stem.bytes().next().is_some_and(|b| b != b'-')
            && stem
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    })
}

/// Rules across files: each main document lists its parts, each part has one `part_of` to
/// its main document, `follows` and `part` give the order of the list (the main list or the
/// lists of the `contents` parts, then the `contents` parts), every part is listed, and every
/// relative link to Markdown or to an assets folder resolves.
fn bundle_rules(root: &Path, mains: &[PathBuf]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut listed_parts = BTreeSet::new();
    for main in mains {
        let shown = main.strip_prefix(root).unwrap().display().to_string();
        let Some(order) = part_order(main, &mut problems) else {
            continue;
        };
        for (k, part) in order.iter().enumerate() {
            let part_shown = part.strip_prefix(root).unwrap().display().to_string();
            let text = std::fs::read_to_string(part).unwrap();
            let relations = relations(&text);
            let part_of: Vec<_> = relations.iter().filter(|(r, _)| r == "part_of").collect();
            match part_of.as_slice() {
                [(_, dest)] if resolve(root, part, dest).as_deref() == Some(main.as_path()) => {}
                other => problems.push(format!(
                    "{part_shown}: part_of is {other:?}, not one link to {shown}"
                )),
            }
            let follows: Vec<_> = relations
                .iter()
                .filter(|(r, _)| r == "follows")
                .map(|(_, d)| resolve(root, part, d))
                .collect();
            let expected: Vec<_> = k
                .checked_sub(1)
                .map(|p| Some(order[p].clone()))
                .into_iter()
                .collect();
            if follows != expected {
                problems.push(format!(
                    "{part_shown}: follows {follows:?}, expected {expected:?}"
                ));
            }
            let fm = split_frontmatter(&text).map(|(y, _)| y).unwrap_or("");
            let key = |name: &str| {
                fm.lines()
                    .find_map(|l| l.strip_prefix(name)?.strip_prefix(": "))
                    .map(str::to_string)
            };
            let want = (Some((k + 1).to_string()), Some(order.len().to_string()));
            if (key("part"), key("parts")) != want {
                problems.push(format!(
                    "{part_shown}: part {:?} of {:?}, expected {want:?}",
                    key("part"),
                    key("parts")
                ));
            }
            listed_parts.insert(part.clone());
        }
    }
    for file in files_under(root) {
        let in_parts = file
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|n| n.to_string_lossy().ends_with(".parts"));
        if in_parts && !listed_parts.contains(&file) {
            problems.push(format!(
                "{}: a part no main document lists",
                file.strip_prefix(root).unwrap().display()
            ));
        }
        if file.extension().is_some_and(|e| e == "md") {
            let text = std::fs::read_to_string(&file).unwrap();
            for (_, dest) in body_links(&text) {
                let path = dest.split('#').next().unwrap();
                let ours = path.ends_with(".md") || path.contains(".assets/");
                if ours && !has_scheme(path) && resolve(root, &file, &dest).is_none() {
                    problems.push(format!(
                        "{}: link {dest} does not resolve",
                        file.strip_prefix(root).unwrap().display()
                    ));
                }
            }
        }
    }
    problems
}

/// The parts of `main` in their order: the list under its `## Parts`, or the lists of the
/// `contents` parts it links, then those `contents` parts. None when it has no parts.
fn part_order(main: &Path, problems: &mut Vec<String>) -> Option<Vec<PathBuf>> {
    let root = main.parent().unwrap();
    let text = std::fs::read_to_string(main).unwrap();
    let section = parts_section(&text)?;
    let mut listed = Vec::new();
    let mut contents = Vec::new();
    let items = list_links(section);
    if items.is_empty() {
        // The list moved to contents parts: follow them from the first one.
        let mut next = body_links(section)
            .into_iter()
            .next()
            .map(|(_, d)| (main.to_path_buf(), d));
        while let Some((from, dest)) = next.take() {
            let Some(file) = resolve(root, &from, &dest) else {
                problems.push(format!("{}: {dest} does not resolve", from.display()));
                break;
            };
            if contents.contains(&file) {
                problems.push(format!("{}: contents parts loop", file.display()));
                break;
            }
            let part = std::fs::read_to_string(&file).unwrap();
            // The first contents part has a `## Parts` heading, the later ones only the
            // list after their title.
            let section = parts_section(&part).unwrap_or_else(|| {
                let body = split_frontmatter(&part).map_or("", |(_, b)| b);
                body.find("\n## Relations\n")
                    .map_or(body, |end| &body[..end])
            });
            for (_, dest) in list_links(section) {
                match resolve(root, &file, &dest) {
                    Some(p) => listed.push(p),
                    None => problems.push(format!("{}: {dest} does not resolve", file.display())),
                }
            }
            next = section
                .lines()
                .filter(|l| l.starts_with("The list goes on in "))
                .flat_map(body_links)
                .next()
                .map(|(_, d)| (file.clone(), d));
            contents.push(file);
        }
    } else {
        for (_, dest) in items {
            match resolve(root, main, &dest) {
                Some(p) => listed.push(p),
                None => problems.push(format!("{}: {dest} does not resolve", main.display())),
            }
        }
    }
    listed.extend(contents);
    Some(listed)
}

/// The `## Parts` section of a document, up to the next `## ` heading.
fn parts_section(text: &str) -> Option<&str> {
    let start = text.find("\n## Parts\n")? + "\n## Parts\n".len();
    let end = text[start..]
        .find("\n## ")
        .map_or(text.len(), |e| start + e + 1);
    Some(&text[start..end])
}

/// The first link of each `- [` list item in `section`.
fn list_links(section: &str) -> Vec<(String, String)> {
    section
        .lines()
        .filter_map(|l| l.strip_prefix("- "))
        .filter(|l| l.starts_with('['))
        .filter_map(|l| body_links(l).into_iter().next())
        .collect()
}

/// The `- <relation> [Title](dest)` lines under the last `## Relations`.
fn relations(text: &str) -> Vec<(String, String)> {
    let Some(at) = text.rfind("\n## Relations\n") else {
        return Vec::new();
    };
    text[at..]
        .lines()
        .filter_map(|l| l.strip_prefix("- "))
        .filter_map(|l| {
            let (word, rest) = l.split_once(' ')?;
            let (_, dest) = body_links(rest).into_iter().next()?;
            Some((word.to_string(), dest))
        })
        .collect()
}

/// The inline links of `text` outside fenced code and code spans: (text, destination).
fn body_links(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        let run = |c: char| trimmed.chars().take_while(|&x| x == c).count();
        if let Some((c, len)) = fence {
            if run(c) >= len && trimmed.trim_end().chars().all(|x| x == c) {
                fence = None;
            }
            continue;
        }
        if let Some(c) = ['`', '~'].into_iter().find(|&c| run(c) >= 3) {
            fence = Some((c, run(c)));
            continue;
        }
        line_links(line, &mut out);
    }
    out
}

/// The `[text](dest)` links of one line, honouring backslash escapes and code spans.
fn line_links(line: &str, out: &mut Vec<(String, String)>) {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '`' => {
                let ticks = chars[i..].iter().take_while(|&&c| c == '`').count();
                let mut j = i + ticks;
                let mut closed = None;
                while j < chars.len() {
                    if chars[j] == '`' {
                        let n = chars[j..].iter().take_while(|&&c| c == '`').count();
                        if n == ticks {
                            closed = Some(j + n);
                            break;
                        }
                        j += n;
                    } else {
                        j += 1;
                    }
                }
                i = closed.unwrap_or(i + ticks);
            }
            '[' => match link_at(&chars, i) {
                Some((text, dest, end)) => {
                    out.push((text, dest));
                    i = end;
                }
                None => i += 1,
            },
            _ => i += 1,
        }
    }
}

/// The link that starts with the `[` at `start`: its text, destination and the index after it.
fn link_at(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    let mut depth = 0usize;
    let mut i = start;
    let close = loop {
        match chars.get(i)? {
            '\\' => i += 1,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    break i;
                }
            }
            _ => {}
        }
        i += 1;
    };
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let text: String = chars[start + 1..close].iter().collect();
    let mut i = close + 2;
    let mut dest = String::new();
    if chars.get(i) == Some(&'<') {
        i += 1;
        loop {
            match chars.get(i)? {
                '>' => break,
                '\\' => {
                    dest.push(*chars.get(i + 1)?);
                    i += 1;
                }
                c => dest.push(*c),
            }
            i += 1;
        }
        i += 1;
    } else {
        while let Some(&c) = chars.get(i) {
            if c == ')' || c.is_whitespace() {
                break;
            }
            if c == '\\' {
                dest.push(*chars.get(i + 1)?);
                i += 2;
                continue;
            }
            dest.push(c);
            i += 1;
        }
    }
    let end = chars[i..].iter().position(|&c| c == ')')? + i + 1;
    Some((text, dest, end))
}

fn has_scheme(dest: &str) -> bool {
    dest.split_once(':').is_some_and(|(scheme, _)| {
        !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    })
}

/// The file the relative link `dest` in `from` names, matching every path component byte
/// for byte (as Crystalline does), never through the file system's folding of case or
/// normalization. None when it names nothing or leaves `root`.
fn resolve(root: &Path, from: &Path, dest: &str) -> Option<PathBuf> {
    let path = dest.split('#').next().unwrap();
    if path.is_empty() || path.starts_with('/') {
        return None;
    }
    let decoded = percent_encoding::percent_decode_str(path)
        .decode_utf8()
        .ok()?;
    let mut at = from.parent().unwrap().to_path_buf();
    for part in decoded.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if at == root {
                    return None;
                }
                at.pop();
            }
            name => {
                let found = std::fs::read_dir(&at)
                    .ok()?
                    .any(|e| e.unwrap().file_name().as_encoded_bytes() == name.as_bytes());
                if !found {
                    return None;
                }
                at.push(name);
            }
        }
    }
    Some(at)
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        if kind.is_dir() {
            out.extend(files_under(&entry.path()));
        } else if kind.is_file() {
            out.push(entry.path());
        }
    }
    out.sort();
    out
}

// ---------------------------------------------------------------------------------------
// The Crystalline import

/// The relative targets of the Markdown links and pictures in `text` outside code, without
/// their fragment and percent-decoded: no scheme, no `#` alone. A target in `<...>` may
/// hold spaces.
fn relative_links(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
        }
        if fence {
            continue;
        }
        // Code spans are left out: a backtick pair and what lies between.
        let plain: String = line.split('`').step_by(2).collect::<Vec<_>>().join(" ");
        let mut rest = plain.as_str();
        while let Some(at) = rest.find("](") {
            rest = &rest[at + 2..];
            // `<target with spaces>` or a target up to `)` or a title.
            let (target, after) = match rest.strip_prefix('<') {
                Some(inner) => {
                    let end = inner.find('>').unwrap_or(inner.len());
                    (&inner[..end], &inner[end..])
                }
                None => {
                    let end = rest.find([')', ' ']).unwrap_or(rest.len());
                    (&rest[..end], &rest[end..])
                }
            };
            rest = after;
            let target = target.split('#').next().unwrap_or("");
            if target.is_empty() || target.contains(':') || target.starts_with('/') {
                continue;
            }
            let decoded = percent_encoding::percent_decode_str(target).decode_utf8_lossy();
            out.push(decoded.into_owned());
        }
    }
    out
}

/// `crystalline` from `PATH`, found without any helper program.
fn crystalline_on_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("crystalline"))
        .find(|file| is_executable(file))
}

#[cfg(unix)]
fn is_executable(file: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(file).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(file: &Path) -> bool {
    file.is_file()
}

/// A `crystalline` binary whose config, state, data and index all live inside one temp folder.
struct Isolated {
    bin: PathBuf,
    root: PathBuf,
    db: PathBuf,
}

impl Isolated {
    fn new(bin: PathBuf, root: &Path) -> Isolated {
        for dir in ["home", "config", "state", "data"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        Isolated {
            bin,
            root: root.to_path_buf(),
            db: root.join("index.db"),
        }
    }

    /// Runs `crystalline <args> --db <index>`; returns stdout, fails with stderr on a
    /// non-zero exit or after [`CALL_TIMEOUT`].
    fn run(&self, args: &[&str]) -> String {
        let (ok, stdout, stderr) = self.call(args, true);
        assert!(ok, "crystalline {}: {stderr}\n{stdout}", args.join(" "));
        stdout
    }

    fn call(&self, args: &[&str], db: bool) -> (bool, String, String) {
        let out = self.root.join("stdout");
        let err = self.root.join("stderr");
        let mut cmd = Command::new(&self.bin);
        cmd.args(args);
        if db {
            cmd.arg("--db").arg(&self.db);
        }
        cmd.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env(
                "CRYSTALLINE_CONFIG",
                self.root.join("config/crystalline.toml"),
            )
            .env("TMPDIR", &self.root)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(std::fs::File::create(&out).unwrap())
            .stderr(std::fs::File::create(&err).unwrap());
        let mut child = cmd.spawn().unwrap();
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if started.elapsed() > CALL_TIMEOUT {
                let _ = child.kill();
                let _ = child.wait();
                panic!("crystalline {} took over {CALL_TIMEOUT:?}", args.join(" "));
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        (
            status.success(),
            std::fs::read_to_string(out).unwrap(),
            std::fs::read_to_string(err).unwrap(),
        )
    }
}

#[test]
fn crystalline_imports_without_errors() {
    let Some(bin) = crystalline_on_path() else {
        println!("skipped: crystalline is not on PATH");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let cr = Isolated::new(bin, &tmp.path().join("crystalline"));
    let (_, version, _) = cr.call(&["--version"], false);
    println!("found {}", version.trim());

    // Both part sizes, each in its own folder of the one import.
    let out = tmp.path().join("out");
    for part_size in part_sizes() {
        let dir = out.join(format!("size-{part_size}"));
        std::fs::create_dir_all(&dir).unwrap();
        write_corpus(&dir, part_size);
    }
    let written: BTreeSet<String> = files_under(&out)
        .iter()
        .filter(|f| f.extension().is_some_and(|e| e == "md"))
        .map(|f| f.strip_prefix(&out).unwrap().to_str().unwrap().to_string())
        .collect();

    let dom = tmp.path().join("dom");
    let dom_arg = dom.to_str().unwrap();
    cr.run(&["domain", "init", dom_arg, "--name", "rezzertest"]);
    cr.run(&["domain", "add", "rezzertest", dom_arg]);
    let import: Value = serde_json::from_str(&cr.run(&[
        "import",
        "--domain",
        "rezzertest",
        "--json",
        out.to_str().unwrap(),
    ]))
    .unwrap();
    println!(
        "import: {} converted, {} copied, {} linked, not taken {}",
        import["files_converted"],
        import["files_copied"],
        import["files_linked"],
        import["not_taken"]
    );

    // Every Markdown file Rezzer wrote got a non-empty permalink, taken from the import's
    // report: the slug may drop letters, so it is never guessed.
    let mut permalinks = BTreeMap::new();
    for file in import["files"].as_array().unwrap() {
        let path = file["path"].as_str().unwrap().to_string();
        let permalink = file["changes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .find_map(|c| c.strip_prefix("permalink: missing -> `")?.strip_suffix('`'))
            .map(str::to_string);
        permalinks.insert(path, permalink);
    }
    let mut problems = Vec::new();
    for path in &written {
        match permalinks.get(path) {
            Some(Some(p)) if !p.is_empty() => {}
            other => problems.push(format!("{path}: permalink {other:?}")),
        }
    }

    let (verified, report, stderr) = cr.call(&["verify", "--json", dom_arg], true);
    let report: Value = serde_json::from_str(&report)
        .unwrap_or_else(|e| panic!("verify gave no JSON ({e}): {stderr}"));
    println!("verify summary: {}", report["summary"]);
    for issue in report["issues"].as_array().unwrap() {
        let path = issue["path"].as_str().unwrap_or("");
        let relative = Path::new(path)
            .strip_prefix(&dom)
            .map_or(path.to_string(), |p| p.display().to_string());
        if relative == "MANIFEST.md" {
            continue;
        }
        let rule = issue["rule"].as_str().unwrap_or("");
        if issue["severity"] == "error" || rule == "L001" || rule == "Q002" {
            problems.push(format!(
                "{relative}:{} {rule} {}: {}",
                issue["line"], issue["severity"], issue["message"]
            ));
        }
    }
    if report["summary"]["errors"] != 0 {
        problems.push(format!("verify summary {}", report["summary"]));
    }
    if !verified {
        problems.push(format!("verify failed: {stderr}"));
    }

    // Every relative link of an imported engram leads to a file in the domain: the import
    // copied each picture and attachment it links (ruling P5-AJ), and rewrote the links.
    let mut attachments = 0;
    for file in files_under(&dom) {
        if file.extension().is_none_or(|e| e != "md") || file.ends_with("MANIFEST.md") {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap();
        for target in relative_links(&text) {
            if target.contains("attachment-") {
                attachments += 1;
            }
            // Crystalline links an attachment from the domain root (`assets/...`).
            let base = if target.starts_with("assets/") {
                dom.as_path()
            } else {
                file.parent().unwrap()
            };
            if !base.join(&target).exists() {
                let relative = file.strip_prefix(&dom).unwrap().display().to_string();
                problems.push(format!("{relative}: link {target} leads to no file"));
            }
        }
    }
    assert!(attachments > 0, "no attachment link was checked");

    // A part of each document with a hard name: its relations resolve after a sync.
    cr.run(&["sync", "--domain", "rezzertest"]);
    let mut read = 0;
    for path in &written {
        let hard = NAMED_COPIES
            .iter()
            .map(|(_, n)| *n)
            .chain([DATA_URI_COPY.1, NFD_NAME])
            .any(|name| {
                let stem = rezzer_pipeline::parts_folder(&rezzer_pipeline::folder_stem(name));
                path.contains(&format!("/{stem}/"))
            });
        if !hard {
            continue;
        }
        let Some(Some(permalink)) = permalinks.get(path) else {
            continue;
        };
        let engram: Value =
            serde_json::from_str(&cr.run(&["read", permalink, "--domain", "rezzertest", "--json"]))
                .unwrap();
        let relations = engram["relations"].as_array().cloned().unwrap_or_default();
        let part_of = relations
            .iter()
            .filter(|r| r["rel_type"] == "part_of" && r["resolved"] == true)
            .count();
        if part_of != 1 {
            problems.push(format!("{path}: {part_of} resolved part_of: {relations:?}"));
        }
        for r in &relations {
            if r["resolved"] != true {
                problems.push(format!("{path}: unresolved relation {r}"));
            }
        }
        read += 1;
    }
    assert!(read >= 6, "only {read} parts with hard names were read");
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}
