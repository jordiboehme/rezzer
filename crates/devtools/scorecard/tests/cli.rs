// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The `scorecard` binary: the check of a set with and without `--names`, the tables it
//! writes, the trace and the summary. All text is invented.

mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::*;

/// Runs the binary with `args`.
fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_scorecard"))
        .args(args)
        .output()
        .expect("the binary runs")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Everything the run printed.
fn printed(out: &Output) -> String {
    format!("{}{}", text(&out.stdout), text(&out.stderr))
}

/// A set of two files: the assertion files under `facts`, the reads under `json`.
fn write_set(root: &Path) {
    let first = document(
        vec![
            table("p1:t1", 0, &[&["Geheimnummer", "4711"]]),
            paragraph("p1:b1", "Vertraulicher Absatz"),
        ],
        vec![
            region("p1:t1", "label_value", "rule", "pdf.strips"),
            region("p1:b1", "spatial", "unsure", "pdf.spatial"),
        ],
        Vec::new(),
    );
    let second = doc(vec![paragraph("p1:b1", "Anderer Text")]);
    let files = [
        (
            "json/Privatbrief.pdf.json",
            serde_json::to_string(&first).unwrap(),
        ),
        (
            "json/ordner/Zweitbrief.pdf.json",
            serde_json::to_string(&second).unwrap(),
        ),
        // A read with no assertion file: its regions still count.
        (
            "json/Drittbrief.pdf.json",
            serde_json::to_string(&first).unwrap(),
        ),
        (
            "facts/Privatbrief.pdf.facts",
            "# Kommentar\npair: Geheimnummer == 4711 [pair]\npair: Geheimnummer == 0815 [pair]\n\
             text: Vertraulicher Absatz [text]\n"
                .to_string(),
        ),
        (
            "facts/ordner/Zweitbrief.pdf.facts",
            "text: Fehlender Wortlaut [text]\n".to_string(),
        ),
        // An assertion file with no read.
        (
            "facts/Viertbrief.pdf.facts",
            "text: Nie gelesen [text]\n".to_string(),
        ),
    ];
    for (path, content) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
}

/// Texts of the facts and names of the files of [`write_set`].
const PRIVATE: [&str; 11] = [
    "Geheimnummer",
    "4711",
    "0815",
    "Vertraulicher",
    "Fehlender",
    "Wortlaut",
    "Kommentar",
    "Privatbrief",
    "Zweitbrief",
    "Viertbrief",
    "ordner",
];

#[test]
fn without_names_the_check_prints_counts_only() {
    let dir = tempfile::tempdir().unwrap();
    write_set(dir.path());
    let facts = dir.path().join("facts");
    let json = dir.path().join("json");
    let out = run(&["check", facts.to_str().unwrap(), json.to_str().unwrap()]);
    assert!(out.status.success(), "{}", printed(&out));
    let all = printed(&out);
    for word in PRIVATE {
        assert!(!all.contains(word), "{word} in:\n{all}");
    }
    // The counts are there, per file by its number and in all.
    let stdout = text(&out.stdout);
    let half_of = |name: &str| text(&run(&["half", name]).stdout).trim().to_string();
    // The reads in the order of their paths: Drittbrief, Privatbrief, ordner/Zweitbrief; the
    // first has no assertion file. A file keeps the number of its read: #2 and #3.
    let first = half_of("Privatbrief.pdf.json");
    let second = half_of("ordner");
    assert!(
        stdout.contains(&format!("#2\t{first}\tpair\t1 / 2\n")),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!("#2\t{first}\ttext\t1 / 1\n")),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!("#3\t{second}\ttext\t0 / 1\n")),
        "{stdout}"
    );
    assert!(stdout.contains("reads\t3\n"), "{stdout}");
    assert!(
        stdout.contains(&format!(
            "assertion files with no read\t{}\t1\n",
            half_of("Viertbrief.pdf.facts")
        )),
        "{stdout}"
    );
    assert!(!stdout.contains("FAIL"), "{stdout}");
}

#[test]
fn with_names_the_check_names_files_and_failing_facts() {
    let dir = tempfile::tempdir().unwrap();
    write_set(dir.path());
    let facts = dir.path().join("facts");
    let json = dir.path().join("json");
    let out = run(&[
        "check",
        "--names",
        facts.to_str().unwrap(),
        json.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", printed(&out));
    let stdout = text(&out.stdout);
    assert!(stdout.contains("Privatbrief.pdf.facts\t"), "{stdout}");
    assert!(
        stdout.contains("  FAIL line 3: pair: Geheimnummer == 0815 [pair]\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("  FAIL line 1: text: Fehlender Wortlaut [text]\n"),
        "{stdout}"
    );
    // A fact that holds is not listed.
    assert!(!stdout.contains("= 4711"), "{stdout}");
}

#[test]
fn an_error_without_names_names_no_file_and_no_text() {
    let dir = tempfile::tempdir().unwrap();
    write_set(dir.path());
    let facts = dir.path().join("facts");
    let json = dir.path().join("json");
    std::fs::write(
        facts.join("Privatbrief.pdf.facts"),
        "pair: Geheimnummer 4711 [pair]\n",
    )
    .unwrap();
    let out = run(&["check", facts.to_str().unwrap(), json.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    let all = printed(&out);
    assert!(all.contains("assertion file #2: line 1: "), "{all}");
    for word in PRIVATE {
        assert!(!all.contains(word), "{word} in:\n{all}");
    }
    // A read that is no model JSON.
    std::fs::write(json.join("Drittbrief.pdf.json"), "{\"Geheimnummer\": 4711}").unwrap();
    let out = run(&["check", facts.to_str().unwrap(), json.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    let all = printed(&out);
    assert!(all.contains("read #1: it is no model JSON"), "{all}");
    for word in PRIVATE {
        assert!(!all.contains(word), "{word} in:\n{all}");
    }
}

#[test]
fn the_tables_hold_every_half_class_and_kind() {
    let dir = tempfile::tempdir().unwrap();
    write_set(dir.path());
    let at = |name: &str| dir.path().join(name).to_str().unwrap().to_string();
    for set in ["erste", "zweite"] {
        let out = run(&[
            "check",
            "--set",
            set,
            "--tsv",
            &at("score.tsv"),
            "--regions-tsv",
            &at("regions.tsv"),
            &at("facts"),
            &at("json"),
        ]);
        assert!(out.status.success(), "{}", printed(&out));
    }
    let score = std::fs::read_to_string(at("score.tsv")).unwrap();
    let lines: Vec<&str> = score.lines().collect();
    // One head, then two sets of two halves of six classes.
    assert_eq!(lines[0], "set\thalf\tclass\tpassed\ttotal\tunread_files");
    // The assertion file with no read is counted in its half, on each of its six lines.
    let unread: u32 = lines[1..13]
        .iter()
        .map(|l| l.rsplit('\t').next().unwrap().parse::<u32>().unwrap())
        .sum();
    assert_eq!(unread, 6);
    assert_eq!(lines.len(), 1 + 2 * 2 * 6);
    let sum = |set: &str, class: &str, field: usize| -> u32 {
        lines
            .iter()
            .map(|l| l.split('\t').collect::<Vec<_>>())
            .filter(|r| r[0] == set && r[2] == class)
            .map(|r| r[field].parse::<u32>().unwrap())
            .sum()
    };
    assert_eq!((sum("erste", "pair", 3), sum("erste", "pair", 4)), (1, 2));
    assert_eq!((sum("zweite", "text", 3), sum("zweite", "text", 4)), (1, 2));
    assert_eq!(sum("erste", "table", 4), 0);

    let regions = std::fs::read_to_string(at("regions.tsv")).unwrap();
    let rows: Vec<Vec<&str>> = regions.lines().map(|l| l.split('\t').collect()).collect();
    assert_eq!(
        rows[0].join("\t"),
        "set\thalf\tkind\tregions\tfacts_passed\tfacts_total"
    );
    // Thirteen kinds and the two places with no region, per half and set.
    assert_eq!(rows.len(), 1 + 2 * 2 * 15);
    let of = |kind: &str, field: usize| -> u32 {
        rows.iter()
            .filter(|r| r[0] == "erste" && r[2] == kind)
            .map(|r| r[field].parse::<u32>().unwrap())
            .sum()
    };
    // The regions of all three reads; the facts of the two with an assertion file.
    assert_eq!(of("label_value", 3), 2);
    assert_eq!(of("spatial", 3), 2);
    assert_eq!((of("label_value", 4), of("label_value", 5)), (1, 2));
    assert_eq!((of("spatial", 4), of("spatial", 5)), (1, 1));
    assert_eq!((of("(not found)", 4), of("(not found)", 5)), (0, 1));

    let out = run(&["summary", "abc1234", &at("score.tsv"), &at("regions.tsv")]);
    assert!(out.status.success(), "{}", printed(&out));
    let summary = text(&out.stdout);
    assert!(summary.starts_with("# Scorecard of abc1234\n"), "{summary}");
    assert!(summary.contains("| erste |  | pair |"), "{summary}");
    assert!(
        summary.contains("| erste |  | **all but link** |"),
        "{summary}"
    );
    assert!(
        summary.contains("### Assertion files with no read"),
        "{summary}"
    );
    assert!(!summary.contains('\u{2014}') && !summary.contains('\u{2013}'));

    // Tables need the name of their set.
    let out = run(&["check", "--tsv", &at("x.tsv"), &at("facts"), &at("json")]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn the_summary_says_what_each_set_is_worth_and_leaves_links_out_of_all() {
    let dir = tempfile::tempdir().unwrap();
    let at = |name: &str| dir.path().join(name).to_str().unwrap().to_string();
    let mut score = String::from("set\thalf\tclass\tpassed\ttotal\tunread_files\n");
    for (set, text, link, unread) in [
        ("briefe", 3, 5, 0),
        ("briefe-untagged", 1, 5, 2),
        ("muster", 2, 0, 0),
    ] {
        score += &format!("{set}\tdev\ttext\t{text}\t4\t{unread}\n");
        score += &format!("{set}\tdev\tlink\t{link}\t{link}\t{unread}\n");
    }
    std::fs::write(at("score.tsv"), score).unwrap();
    std::fs::write(
        at("regions.tsv"),
        "set\thalf\tkind\tregions\tfacts_passed\tfacts_total\nbriefe\tdev\ttable\t1\t1\t1\n",
    )
    .unwrap();
    let out = run(&["summary", "abc1234", &at("score.tsv"), &at("regions.tsv")]);
    assert!(out.status.success(), "{}", printed(&out));
    let summary = text(&out.stdout);
    for line in [
        "| briefe | drift check (truth from Rezzer's own tagged read) | text | 3 / 4 (75.0 %) |",
        "| briefe-untagged | lower bound | text | 1 / 4 (25.0 %) |",
        "| muster |  | text | 2 / 4 (50.0 %) |",
        // Five of five links hold and are in no `all`.
        "| briefe-untagged | lower bound | link | 5 / 5 (100.0 %) |",
        "| briefe-untagged | lower bound | **all but link** | 1 / 4 (25.0 %) |",
        "| briefe-untagged | 2 | 0 |",
        "check that the data is present",
    ] {
        assert!(summary.contains(line), "{line} not in:\n{summary}");
    }
}

#[test]
fn an_internal_error_without_names_prints_its_file_number_and_no_text() {
    let dir = tempfile::tempdir().unwrap();
    write_set(dir.path());
    let facts = dir.path().join("facts");
    let json = dir.path().join("json");
    let panic_at = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_scorecard"))
            .args(args)
            .env("SCORECARD_PANIC_AT", "2")
            .output()
            .expect("the binary runs")
    };
    let out = panic_at(&["check", facts.to_str().unwrap(), json.to_str().unwrap()]);
    assert!(!out.status.success());
    assert_eq!(text(&out.stderr), "scorecard: internal error in file #2\n");
    let all = printed(&out);
    for word in PRIVATE {
        assert!(!all.contains(word), "{word} in:\n{all}");
    }
    // Every other command is as quiet: a file that is no model JSON gives an error with its
    // reason, and no command prints a panic message of Rust.
    for args in [&["gen", "tagged"][..], &["trace"], &["gen", "fields"]] {
        let mut args = args.to_vec();
        let read = json.join("Privatbrief.pdf.json");
        args.push(read.to_str().unwrap());
        if args[0] == "trace" {
            args.push("Geheimnummer");
        }
        let out = panic_at(&args);
        assert!(!text(&out.stderr).contains("panicked"), "{}", printed(&out));
    }
    // With names the message of the panic is printed, as for any public file.
    let out = panic_at(&[
        "check",
        "--names",
        facts.to_str().unwrap(),
        json.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("Privatbrief"),
        "{}",
        printed(&out)
    );
}

#[test]
fn the_trace_prints_the_path_and_no_text() {
    let dir = tempfile::tempdir().unwrap();
    let doc = document(
        vec![
            table("p1:t1", 0, &[&["Geheimnummer", "4711"]]),
            paragraph("p1:b1", "Vertraulicher Absatz"),
            paragraph("p1:b2", "Absatz ohne Region"),
        ],
        vec![
            region("p1:t1", "label_value", "rule", "pdf.strips"),
            region("p1:b1", "paragraph", "exact", "given.file_states"),
        ],
        Vec::new(),
    );
    let json = dir.path().join("a.json");
    std::fs::write(&json, serde_json::to_string(&doc).unwrap()).unwrap();
    let json = json.to_str().unwrap();

    let by_text = run(&["trace", json, "Geheimnummer"]);
    assert_eq!(
        text(&by_text.stdout),
        "p1:t1\tpage 1\tlabel_value\trule\tscore 1000\tevidence pdf.strips\tbounds 10 20 110.5 \
         220.25\talso -\n"
    );
    let by_anchor = run(&["trace", json, "p1:b1"]);
    assert_eq!(
        text(&by_anchor.stdout),
        "p1:b1\tpage 1\tparagraph\texact\tscore 1000\tevidence given.file_states\tbounds 10 20 \
         110.5 220.25\talso -\n"
    );
    // A text in two blocks gives both; a block with no region says so.
    let both = run(&["trace", json, "Absatz"]);
    let lines = text(&both.stdout);
    assert_eq!(lines.lines().count(), 2, "{lines}");
    assert!(lines.ends_with("no region\n"), "{lines}");
    for out in [&by_text, &by_anchor, &both] {
        let all = printed(out);
        for word in ["Geheimnummer", "4711", "Vertraulicher", "Wortlaut", "ohne"] {
            assert!(!all.contains(word), "{word} in:\n{all}");
        }
    }
    let none = run(&["trace", json, "Fehlt"]);
    assert_eq!(none.status.code(), Some(1));
    assert_eq!(text(&none.stderr), "scorecard: not found\n");
}

#[test]
fn a_wrong_call_prints_the_usage() {
    for args in [
        &[][..],
        &["check"],
        &["check", "--nmes", "a", "b"],
        &["gen", "tagged"],
    ] {
        let out = run(args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(text(&out.stderr).starts_with("usage: scorecard check"));
    }
}
