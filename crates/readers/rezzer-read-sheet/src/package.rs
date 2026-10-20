// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The package checks that run before calamine sees the bytes.
//!
//! calamine resolves parts by its own rules (last officeDocument relationship, last
//! relationship of an Id, first sheet of a name, literal path joins, case-insensitive entry
//! lookup). Rather than mirror them, every entry is measured, and a package whose structure
//! could be read two ways is refused: valid Excel files never have that.

use std::collections::{HashMap, HashSet};

use quick_xml::Reader;
use quick_xml::events::Event;
use rezzer_limits::{Budget, LimitHit};
use rezzer_model::LossReason;
use sheetparts::ooxml::WorkbookParts;
use zipguard::{Archive, ZipError};

/// The measured sizes of all entries, by lowercase name with `\` read as `/` (the way
/// calamine looks entries up).
pub(crate) struct Package {
    sizes: HashMap<String, u64>,
}

/// Why the package is not read: the reason and the text of the loss.
pub(crate) type Refusal = (LossReason, String);

pub(crate) fn zip_reason(e: &ZipError) -> LossReason {
    match e {
        ZipError::Limit { .. } => LossReason::Limit,
        ZipError::Encrypted(_) => LossReason::Encrypted,
        _ => LossReason::Damaged,
    }
}

fn key(name: &str) -> String {
    name.replace('\\', "/").to_ascii_lowercase()
}

impl Package {
    /// Measures every entry of the archive (entry size and ratio through zipguard) and
    /// charges it to the budget. Entries whose names differ only in case are refused.
    pub fn preflight(zip: &mut Archive, budget: &Budget) -> Result<Package, Refusal> {
        let names: Vec<String> = zip.names().iter().map(|n| n.to_string()).collect();
        let mut sizes = HashMap::new();
        let mut seen: HashMap<String, String> = HashMap::new();
        for name in &names {
            let k = key(name);
            if let Some(other) = seen.insert(k.clone(), name.clone()) {
                return Err((
                    LossReason::Damaged,
                    format!("the entries {other} and {name} differ only in case or slashes"),
                ));
            }
            if name.ends_with('/') {
                continue;
            }
            let n = zip
                .measure(name)
                .map_err(|e| (zip_reason(&e), format!("entry {name}: {e}")))?;
            budget
                .add_unpacked(n)
                .map_err(|hit| (LossReason::Limit, format!("entry {name}: {hit}")))?;
            sizes.insert(k, n);
        }
        Ok(Package { sizes })
    }

    /// The measured size of a part as calamine looks it up; 0 when it is not there.
    pub fn size(&self, part: &str) -> u64 {
        self.sizes.get(&key(part)).copied().unwrap_or(0)
    }

    /// Charges one more unpacking of `part` (calamine reads parts again) to the budget.
    pub fn charge(&self, budget: &Budget, part: &str) -> Result<(), LimitHit> {
        budget.add_unpacked(self.size(part))
    }
}

/// One relationship as written (attribute values not unescaped).
struct Rel {
    id: String,
    kind: String,
    target: String,
    external: bool,
}

/// Reads the relationships of a `.rels` part. A DOCTYPE or broken XML is a refusal.
fn rels(zip: &mut Archive, part: &str) -> Result<Vec<Rel>, Refusal> {
    let bytes = zip
        .read(part)
        .map_err(|e| (zip_reason(&e), format!("{part}: {e}")))?;
    let mut reader = Reader::from_reader(bytes.as_slice());
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let damaged = |what: String| (LossReason::Damaged, format!("{part}: {what}"));
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e) | Event::Empty(e)) if e.local_name().as_ref() == "Relationship" => {
                let mut rel = Rel {
                    id: String::new(),
                    kind: String::new(),
                    target: String::new(),
                    external: false,
                };
                for a in e.attributes() {
                    let a = a.map_err(|e| damaged(e.to_string()))?;
                    let value = a.value.to_string();
                    match a.key.local_name().as_ref() {
                        "Id" => rel.id = value,
                        "Type" => rel.kind = value,
                        "Target" => rel.target = value,
                        "TargetMode" => rel.external = value.eq_ignore_ascii_case("External"),
                        _ => {}
                    }
                }
                out.push(rel);
            }
            Ok(Event::DocType(_)) => return Err(damaged("a DOCTYPE is not allowed".into())),
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(damaged(e.to_string())),
        }
    }
    let mut ids = HashSet::new();
    for r in &out {
        if !ids.insert(r.id.as_str()) {
            return Err(damaged(format!(
                "the relationship id {} is used twice",
                r.id
            )));
        }
    }
    Ok(out)
}

/// A target both calamine and sheetparts read the same way: no `.`, `..` or empty
/// segments, no backslash, no escaped characters.
fn clean(target: &str) -> bool {
    let path = target.strip_prefix('/').unwrap_or(target);
    !path.is_empty()
        && !target.contains(['\\', '&'])
        && path
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..")
}

/// The part a target names, as calamine joins it (absolute, or after `dir`).
fn join(dir: &str, target: &str) -> String {
    match target.strip_prefix('/') {
        Some(abs) => abs.to_string(),
        None => format!("{dir}{target}"),
    }
}

/// Refuses a package that calamine and sheetparts could read in two ways: duplicate
/// relationship ids in any `.rels` part, not exactly one officeDocument relationship,
/// unclean or external workbook and sheet targets, parts that are not there, and sheet
/// names that repeat (case-insensitive).
pub(crate) fn check_structure(zip: &mut Archive, wb: &WorkbookParts) -> Result<(), Refusal> {
    let damaged = |what: String| (LossReason::Damaged, what);
    let names: Vec<String> = zip.names().iter().map(|n| n.to_string()).collect();
    let exists: HashSet<&str> = names.iter().map(String::as_str).collect();
    let mut parsed: HashMap<String, Vec<Rel>> = HashMap::new();
    for name in names.iter().filter(|n| n.ends_with(".rels")) {
        parsed.insert(name.clone(), rels(zip, name)?);
    }

    let root = parsed.get("_rels/.rels").map(Vec::as_slice).unwrap_or(&[]);
    let office: Vec<&Rel> = root
        .iter()
        .filter(|r| r.kind.ends_with("/officeDocument"))
        .collect();
    if office.len() != 1 {
        return Err(damaged(format!(
            "_rels/.rels has {} officeDocument relationships, not one",
            office.len()
        )));
    }
    let doc = office[0];
    if doc.external || !clean(&doc.target) {
        return Err(damaged(format!(
            "the workbook target {:?} is external or not a plain path",
            doc.target
        )));
    }
    let part = join("", &doc.target);
    if part != wb.workbook_part || !exists.contains(part.as_str()) {
        return Err(damaged(format!(
            "the workbook part {part} is not in the package"
        )));
    }
    let dir = match part.rsplit_once('/') {
        Some((d, _)) => format!("{d}/"),
        None => String::new(),
    };
    if part != format!("{dir}workbook.xml") {
        return Err(damaged(format!(
            "the workbook part is {part}, not {dir}workbook.xml"
        )));
    }

    let wb_rels = format!("{dir}_rels/workbook.xml.rels");
    for r in parsed.get(&wb_rels).map(Vec::as_slice).unwrap_or(&[]) {
        let sheet_like = ["/worksheet", "/chartsheet", "/dialogsheet"]
            .iter()
            .any(|k| r.kind.ends_with(k));
        if !sheet_like {
            continue;
        }
        if r.external || !clean(&r.target) {
            return Err(damaged(format!(
                "the sheet target {:?} is external or not a plain path",
                r.target
            )));
        }
        let p = join(&dir, &r.target);
        if !exists.contains(p.as_str()) {
            return Err(damaged(format!("the sheet part {p} is not in the package")));
        }
    }

    let mut sheet_names = HashSet::new();
    for s in &wb.sheets {
        if !sheet_names.insert(s.name.to_lowercase()) {
            return Err(damaged(format!(
                "the sheet name {:?} is used twice",
                s.name
            )));
        }
        if s.part.is_empty() || !exists.contains(s.part.as_str()) {
            return Err(damaged(format!(
                "sheet {:?} has no part in the package",
                s.name
            )));
        }
    }
    Ok(())
}
