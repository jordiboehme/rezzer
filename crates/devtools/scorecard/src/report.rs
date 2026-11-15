// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The score of a set: its assertion files checked against the model JSON of its reads, per
//! file, per half and per class, and the regions of its reads per kind with the facts that
//! stand on them.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use rezzer_model::{Document, RegionKind};

use crate::check::{Place, RegionKindName, check, kind_name};
use crate::doc::View;
use crate::facts::{Class, Written, parse};
use crate::split::{Half, half, key};

/// How many facts passed, of how many.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub passed: u32,
    pub total: u32,
}

impl Counts {
    fn add(&mut self, pass: bool) {
        self.total += 1;
        self.passed += u32::from(pass);
    }

    fn sum(&mut self, other: Counts) {
        self.total += other.total;
        self.passed += other.passed;
    }
}

/// The score of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileScore {
    /// The path of its assertion file below the set.
    pub name: String,
    /// The place of its read among the reads of the set in the order of their paths, from 1:
    /// the `#n` a file is called by without names, in the report and in an error alike.
    pub number: usize,
    pub half: Half,
    pub classes: BTreeMap<Class, Counts>,
    /// The facts by where their text stands in the read.
    pub places: BTreeMap<Place, Counts>,
    /// The failing facts: the line number in the assertion file and the line.
    pub failing: Vec<(usize, String)>,
}

/// The read that is checked now, as its `#n`; 0 when none is. For the message of an internal
/// error, which must name no file and quote no text (see the binary's panic hook).
pub static CURRENT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A self-test of the panic hook: with this variable set to `n`, the check panics at read
/// `#n` with a message that holds the read's path and text, as a slip in the code could.
const PANIC_AT: &str = "SCORECARD_PANIC_AT";

/// Checks `facts` against `doc`. A fact that stands more than once in the assertion file
/// counts once.
pub fn score_file(
    name: &str,
    number: usize,
    half: Half,
    facts: &[Written],
    doc: &Document,
) -> FileScore {
    let view = View::of(doc);
    let mut score = FileScore {
        name: name.to_string(),
        number,
        half,
        classes: BTreeMap::new(),
        places: BTreeMap::new(),
        failing: Vec::new(),
    };
    let mut seen: Vec<&crate::facts::Fact> = Vec::new();
    for written in facts {
        if seen.contains(&&written.fact) {
            continue;
        }
        seen.push(&written.fact);
        let outcome = check(&view, &written.fact);
        score
            .classes
            .entry(written.fact.class)
            .or_default()
            .add(outcome.pass);
        score
            .places
            .entry(outcome.place)
            .or_default()
            .add(outcome.pass);
        if !outcome.pass {
            score.failing.push((written.number, written.line.clone()));
        }
    }
    score
}

/// Every region kind, in the order of the model.
const KINDS: [RegionKind; 13] = [
    RegionKind::HeaderBand,
    RegionKind::FooterBand,
    RegionKind::TitleBlock,
    RegionKind::Heading,
    RegionKind::Paragraph,
    RegionKind::List,
    RegionKind::Table,
    RegionKind::LabelValue,
    RegionKind::Totals,
    RegionKind::Figure,
    RegionKind::Code,
    RegionKind::FootnoteLegend,
    RegionKind::Spatial,
];

/// The score of a set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetScore {
    /// The files with an assertion file and a read, in the order of their paths.
    pub files: Vec<FileScore>,
    /// Assertion files whose read is missing, by half. Their facts are in no count, so a set
    /// with one of them is not whole: the tables carry the number and `scorecard.sh` fails.
    pub unread: BTreeMap<Half, usize>,
    /// Reads of the set, with or without an assertion file.
    pub reads: usize,
    /// The regions of every read of the set, by half and kind.
    pub regions: BTreeMap<(Half, &'static str), u32>,
}

/// The files under `dir` whose name ends with `ending`, as paths below `dir`, sorted.
fn files_under(dir: &Path, ending: &str) -> Result<BTreeSet<PathBuf>, String> {
    fn walk(
        root: &Path,
        dir: &Path,
        ending: &str,
        out: &mut BTreeSet<PathBuf>,
    ) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let kind = entry.file_type()?;
            if kind.is_dir() {
                walk(root, &path, ending, out)?;
            } else if kind.is_file() && path.to_string_lossy().ends_with(ending) {
                out.insert(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
            }
        }
        Ok(())
    }
    let mut out = BTreeSet::new();
    // The folder is named: it is an argument of the caller, not a file of the set.
    walk(dir, dir, ending, &mut out).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(out)
}

/// `path` with `from` at its end replaced by `to`.
fn with_ending(path: &Path, from: &str, to: &str) -> PathBuf {
    let text = path.to_string_lossy();
    PathBuf::from(format!("{}{to}", text.strip_suffix(from).unwrap_or(&text)))
}

/// Scores the assertion files (`*.facts`) under `facts_dir` against the reads (`*.json`, the
/// model JSON) under `json_dir`: the facts of `a/b.pdf.facts` are checked on `a/b.pdf.json`.
/// With `names` an error names the file; without, only its place in the order of the paths.
pub fn score_set(facts_dir: &Path, json_dir: &Path, names: bool) -> Result<SetScore, String> {
    let facts = files_under(facts_dir, ".facts")?;
    let reads = files_under(json_dir, ".json")?;
    let mut score = SetScore {
        reads: reads.len(),
        ..SetScore::default()
    };
    for kind in KINDS {
        for h in Half::ALL {
            score.regions.insert((h, kind_name(kind)), 0);
        }
    }
    let label = |n: usize, path: &Path| {
        if names {
            path.display().to_string()
        } else {
            format!("#{}", n + 1)
        }
    };
    for h in Half::ALL {
        score.unread.insert(h, 0);
    }
    for (n, read) in reads.iter().enumerate() {
        CURRENT.store(n + 1, std::sync::atomic::Ordering::Relaxed);
        let failed = |what: &str, why: String| format!("{} {}: {why}", what, label(n, read));
        let text = std::fs::read_to_string(json_dir.join(read))
            .map_err(|e| failed("read", e.to_string()))?;
        // The reason of serde may quote the file's text: it is given only with names.
        let doc: Document = serde_json::from_str(&text).map_err(|e| {
            let why = if names {
                e.to_string()
            } else {
                format!("it is no model JSON (line {})", e.line())
            };
            failed("read", why)
        })?;
        if std::env::var(PANIC_AT).is_ok_and(|at| at == (n + 1).to_string()) {
            panic!("asked for at read {}: {text}", read.display());
        }
        let h = half(&key(read));
        for region in &doc.regions {
            *score
                .regions
                .entry((h, kind_name(region.kind)))
                .or_insert(0) += 1;
        }
        let own = with_ending(read, ".json", ".facts");
        if !facts.contains(&own) {
            continue;
        }
        let text = std::fs::read_to_string(facts_dir.join(&own))
            .map_err(|e| failed("assertion file", e.to_string()))?;
        let written = parse(&text).map_err(|why| failed("assertion file", why))?;
        score
            .files
            .push(score_file(&own.to_string_lossy(), n + 1, h, &written, &doc));
    }
    CURRENT.store(0, std::sync::atomic::Ordering::Relaxed);
    for unread in facts
        .iter()
        .filter(|f| !reads.contains(&with_ending(f, ".facts", ".json")))
    {
        *score.unread.entry(half(&key(unread))).or_insert(0) += 1;
    }
    Ok(score)
}

impl SetScore {
    /// The facts of the set by half and class; every class of every half has a line.
    pub fn totals(&self) -> BTreeMap<(Half, Class), Counts> {
        let mut totals = BTreeMap::new();
        for h in Half::ALL {
            for class in Class::ALL {
                totals.insert((h, class), Counts::default());
            }
        }
        for file in &self.files {
            for (class, counts) in &file.classes {
                totals
                    .entry((file.half, *class))
                    .or_insert_with(Counts::default)
                    .sum(*counts);
            }
        }
        totals
    }

    /// The facts of the set by half and by where their text stands.
    fn places(&self) -> BTreeMap<(Half, Place), Counts> {
        let mut places: BTreeMap<(Half, Place), Counts> = BTreeMap::new();
        for file in &self.files {
            for (place, counts) in &file.places {
                places.entry((file.half, *place)).or_default().sum(*counts);
            }
        }
        places
    }

    /// The report: per file and class `passed / total`, then the totals per half and class,
    /// the regions per half, and the assertion files with no read. With `names` each file is
    /// named and its failing facts are listed. Without, a file is `#n` by the order of the
    /// paths of all reads and no text of a fact or of a file is printed.
    pub fn report(&self, names: bool) -> String {
        let mut out = String::from("file\thalf\tclass\tpassed / total\n");
        for file in &self.files {
            let label = if names {
                file.name.clone()
            } else {
                format!("#{}", file.number)
            };
            for (class, c) in &file.classes {
                let _ = writeln!(
                    out,
                    "{label}\t{}\t{}\t{} / {}",
                    file.half.name(),
                    class.name(),
                    c.passed,
                    c.total
                );
            }
            if names {
                for (number, line) in &file.failing {
                    let _ = writeln!(out, "  FAIL line {number}: {line}");
                }
            }
        }
        for ((h, class), c) in self.totals() {
            let _ = writeln!(
                out,
                "total\t{}\t{}\t{} / {}",
                h.name(),
                class.name(),
                c.passed,
                c.total
            );
        }
        for h in Half::ALL {
            let count = |want: bool| -> u32 {
                self.regions
                    .iter()
                    .filter(|((half, kind), _)| *half == h && (*kind == "spatial") == want)
                    .map(|(_, n)| *n)
                    .sum()
            };
            let (typed, spatial) = (count(false), count(true));
            let _ = writeln!(
                out,
                "regions\t{}\ttyped {typed} / {}, spatial {spatial}",
                h.name(),
                typed + spatial
            );
        }
        let _ = writeln!(out, "reads\t{}", self.reads);
        for h in Half::ALL {
            let _ = writeln!(
                out,
                "assertion files with no read\t{}\t{}",
                h.name(),
                self.unread.get(&h).copied().unwrap_or(0)
            );
        }
        out
    }

    /// The lines of the scorecard TSV: `set  half  class  passed  total  unread_files`. The
    /// last field is the same on every line of a half: its assertion files with no read.
    pub fn score_rows(&self, set: &str) -> String {
        let mut out = String::new();
        for ((h, class), c) in self.totals() {
            let _ = writeln!(
                out,
                "{set}\t{}\t{}\t{}\t{}\t{}",
                h.name(),
                class.name(),
                c.passed,
                c.total,
                self.unread.get(&h).copied().unwrap_or(0)
            );
        }
        out
    }

    /// The lines of the regions TSV: `set  half  kind  regions  facts_passed  facts_total`.
    /// The facts of a kind are those whose text stands in a region of that kind; `(no
    /// region)` and `(not found)` hold the facts that stand in none.
    pub fn region_rows(&self, set: &str) -> String {
        let places = self.places();
        let mut out = String::new();
        for h in Half::ALL {
            let mut line = |name: &str, regions: u32, place: Place| {
                let c = places.get(&(h, place)).copied().unwrap_or_default();
                let _ = writeln!(
                    out,
                    "{set}\t{}\t{name}\t{regions}\t{}\t{}",
                    h.name(),
                    c.passed,
                    c.total
                );
            };
            for kind in KINDS {
                let name = kind_name(kind);
                let regions = self.regions.get(&(h, name)).copied().unwrap_or(0);
                line(name, regions, Place::Region(RegionKindName(name)));
            }
            line(Place::NoRegion.name(), 0, Place::NoRegion);
            line(Place::NotFound.name(), 0, Place::NotFound);
        }
        out
    }
}

/// The first line of the scorecard TSV.
pub const SCORE_HEAD: &str = "set\thalf\tclass\tpassed\ttotal\tunread_files";

/// The first line of the regions TSV.
pub const REGION_HEAD: &str = "set\thalf\tkind\tregions\tfacts_passed\tfacts_total";

/// `passed / total` with the share in percent, or a dash for no facts.
fn share(passed: u64, total: u64) -> String {
    if total == 0 {
        "-".to_string()
    } else {
        format!(
            "{passed} / {total} ({:.1} %)",
            passed as f64 * 100.0 / total as f64
        )
    }
}

/// The fields of the lines of a TSV after its first line; a line with too few fields or a
/// count that is no number is an error.
fn rows(tsv: &str, fields: usize) -> Result<Vec<Vec<&str>>, String> {
    tsv.lines()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let row: Vec<&str> = l.split('\t').collect();
            if row.len() == fields && row[3..].iter().all(|n| n.parse::<u64>().is_ok()) {
                Ok(row)
            } else {
                Err(format!("not a line of {fields} fields with counts: {l}"))
            }
        })
        .collect()
}

/// What the facts of a set are worth, by its name and the other sets of the table: a set
/// `X-untagged` is read without the tags its truth comes from; its sister `X` is read with
/// them.
fn worth(set: &str, sets: &[&str]) -> &'static str {
    if set.ends_with("-untagged") {
        "lower bound"
    } else if sets.contains(&format!("{set}-untagged").as_str()) {
        "drift check (truth from Rezzer's own tagged read)"
    } else {
        ""
    }
}

/// The Markdown summary of a scorecard TSV and its regions TSV, for the commit `title` names.
pub fn summary(title: &str, score_tsv: &str, regions_tsv: &str) -> Result<String, String> {
    let n = |s: &str| s.parse::<u64>().unwrap_or(0);
    let mut out = format!("# Scorecard of {title}\n\n");
    out.push_str(
        "Facts that hold, of the facts checked, per set, half and class. `dev` files may be \
         named in a task; `held` files are held out.\n\n\
         How to read the sets:\n\n\
         - **drift check (truth from Rezzer's own tagged read)**: the truth of these sets was \
         made from Rezzer's read of the files with their tags. They hold by construction on \
         the day the truth is made. A lower number later shows that the tagged read moved. \
         They are no evidence that a read is right.\n\
         - **lower bound**: the same files read with their tags taken out, against that truth. \
         Text extraction is shared with the truth, and tag conventions that the layout does \
         not show count as failures. The numbers are a lower bound on structure, not a proof \
         of right values.\n\
         - Facts from invoice XML check that the data is present and that an item line is one \
         row. They do not check which label an amount stands with.\n\
         - `all` leaves out the `link` class: links come from annotations, not from layout.\n\
         - A set with assertion files that have no read is not whole: their facts are in no \
         count.\n\n",
    );
    // set -> class -> half -> (passed, total)
    let mut sets: Vec<&str> = Vec::new();
    let mut score: BTreeMap<(&str, &str, &str), (u64, u64)> = BTreeMap::new();
    let mut unread: BTreeMap<(&str, &str), u64> = BTreeMap::new();
    for row in rows(score_tsv, 6)? {
        if !sets.contains(&row[0]) {
            sets.push(row[0]);
        }
        score.insert((row[0], row[2], row[1]), (n(row[3]), n(row[4])));
        unread.insert((row[0], row[1]), n(row[5]));
    }
    out.push_str("| set | what it is | class | dev | held |\n|---|---|---|---:|---:|\n");
    for set in &sets {
        let what = worth(set, &sets);
        let (mut dev, mut held) = ((0, 0), (0, 0));
        for class in Class::ALL {
            let d = score
                .get(&(set, class.name(), "dev"))
                .copied()
                .unwrap_or((0, 0));
            let h = score
                .get(&(set, class.name(), "held"))
                .copied()
                .unwrap_or((0, 0));
            if class != Class::Link {
                dev = (dev.0 + d.0, dev.1 + d.1);
                held = (held.0 + h.0, held.1 + h.1);
            }
            if d.1 + h.1 > 0 {
                let _ = writeln!(
                    out,
                    "| {set} | {what} | {} | {} | {} |",
                    class.name(),
                    share(d.0, d.1),
                    share(h.0, h.1)
                );
            }
        }
        let _ = writeln!(
            out,
            "| {set} | {what} | **all but link** | {} | {} |",
            share(dev.0, dev.1),
            share(held.0, held.1)
        );
    }
    out.push_str("\n### Assertion files with no read\n\n| set | dev | held |\n|---|---:|---:|\n");
    for set in &sets {
        let of = |h: &str| unread.get(&(*set, h)).copied().unwrap_or(0);
        let _ = writeln!(out, "| {set} | {} | {} |", of("dev"), of("held"));
    }
    out.push_str(
        "\n## Regions\n\nRegions with a type, of all regions, and the regions kept as laid out \
         (`spatial`). Facts on typed regions: the facts whose text stands in a region with a \
         type, and how many of them hold.\n\n",
    );
    out.push_str(
        "| set | half | typed regions | spatial | facts on typed regions | facts on spatial \
         regions | facts with no region | facts whose text is not found |\n\
         |---|---|---:|---:|---:|---:|---:|---:|\n",
    );
    let region_rows = rows(regions_tsv, 6)?;
    let mut seen: Vec<(&str, &str)> = Vec::new();
    for row in &region_rows {
        if !seen.contains(&(row[0], row[1])) {
            seen.push((row[0], row[1]));
        }
    }
    for (set, h) in seen {
        let of = |want: &dyn Fn(&str) -> bool| {
            region_rows
                .iter()
                .filter(|r| r[0] == set && r[1] == h && want(r[2]))
                .fold((0, 0, 0), |sum, r| {
                    (sum.0 + n(r[3]), sum.1 + n(r[4]), sum.2 + n(r[5]))
                })
        };
        let typed = of(&|k| k != "spatial" && !k.starts_with('('));
        let spatial = of(&|k| k == "spatial");
        let none = of(&|k| k == Place::NoRegion.name());
        let lost = of(&|k| k == Place::NotFound.name());
        let all = typed.0 + spatial.0;
        let _ = writeln!(
            out,
            "| {set} | {h} | {} | {} | {} | {} | {} | {} |",
            share(typed.0, all),
            share(spatial.0, all),
            share(typed.1, typed.2),
            share(spatial.1, spatial.2),
            share(none.1, none.2),
            share(lost.1, lost.2)
        );
    }
    out.push_str("\n### Per kind\n\n| set | half | kind | regions | facts that hold |\n|---|---|---|---:|---:|\n");
    for row in &region_rows {
        if n(row[3]) + n(row[5]) > 0 {
            let _ = writeln!(
                out,
                "| {} | {} | {} | {} | {} |",
                row[0],
                row[1],
                row[2],
                row[3],
                share(n(row[4]), n(row[5]))
            );
        }
    }
    Ok(out)
}
