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

    /// Whether the package has this entry, looked up as calamine does.
    pub fn contains(&self, part: &str) -> bool {
        self.sizes.contains_key(&key(part))
    }

    /// Charges one more unpacking of `part` (calamine reads parts again) to the budget.
    pub fn charge(&self, budget: &Budget, part: &str) -> Result<(), LimitHit> {
        budget.add_unpacked(self.size(part))
    }
}

/// One relationship as written (attribute values not unescaped).
pub(crate) struct Rel {
    pub id: String,
    pub kind: String,
    pub target: String,
    pub external: bool,
}

/// The namespace of package relationship parts.
const PKG_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

/// The namespaces of the `r:id` attribute (transitional and strict OOXML).
const REL_NS: [&str; 2] = [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
    "http://purl.oclc.org/ooxml/officeDocument/relationships",
];

/// Reads the relationships of a `.rels` part. Only canonical markup is accepted, the kind
/// every parser reads the same way: a `Relationships` root with only the package namespace
/// as its default namespace, and `Relationship` children with exactly the unprefixed
/// attributes `Id`, `Type`, `Target` and optionally `TargetMode`, each once. A DOCTYPE,
/// other elements or attributes, or broken XML are a refusal.
fn rels(zip: &mut Archive, part: &str) -> Result<Vec<Rel>, Refusal> {
    let bytes = zip
        .read(part)
        .map_err(|e| (zip_reason(&e), format!("{part}: {e}")))?;
    let mut reader = Reader::from_reader(bytes.as_slice());
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let damaged = |what: String| (LossReason::Damaged, format!("{part}: {what}"));
    let mut depth = 0usize;
    let mut root_seen = false;
    loop {
        buf.clear();
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| damaged(e.to_string()))?;
        let (e, empty) = match event {
            Event::Start(e) => (e, false),
            Event::Empty(e) => (e, true),
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                continue;
            }
            Event::Text(t) => {
                if !t.as_ref().chars().all(char::is_whitespace) {
                    return Err(damaged("text outside the relationships".into()));
                }
                continue;
            }
            Event::DocType(_) => return Err(damaged("a DOCTYPE is not allowed".into())),
            Event::Eof => break,
            _ => continue,
        };
        let name = e.name().as_ref().to_string();
        let mut attrs: Vec<(String, String)> = Vec::new();
        for a in e.attributes() {
            let a = a.map_err(|e| damaged(e.to_string()))?;
            attrs.push((a.key.as_ref().to_string(), a.value.to_string()));
        }
        match (depth, name.as_str()) {
            (0, "Relationships") if !root_seen => {
                root_seen = true;
                if attrs.len() != 1 || attrs[0].0 != "xmlns" || attrs[0].1 != PKG_NS {
                    return Err(damaged(
                        "the root must carry only the package relationships namespace".into(),
                    ));
                }
            }
            (1, "Relationship") => {
                let mut rel = Rel {
                    id: String::new(),
                    kind: String::new(),
                    target: String::new(),
                    external: false,
                };
                let mut seen = HashSet::new();
                for (key, value) in attrs {
                    if !seen.insert(key.clone()) {
                        return Err(damaged(format!("the attribute {key} is repeated")));
                    }
                    match key.as_str() {
                        "Id" => rel.id = value,
                        "Type" => rel.kind = value,
                        "Target" => rel.target = value,
                        "TargetMode" => rel.external = value.eq_ignore_ascii_case("External"),
                        other => {
                            return Err(damaged(format!(
                                "a relationship has the attribute {other}"
                            )));
                        }
                    }
                }
                if !["Id", "Type", "Target"].iter().all(|k| seen.contains(*k)) {
                    return Err(damaged(
                        "a relationship lacks its Id, Type or Target".into(),
                    ));
                }
                out.push(rel);
            }
            _ => return Err(damaged(format!("unexpected element {name}"))),
        }
        if !empty {
            depth += 1;
        }
    }
    if !root_seen {
        return Err(damaged("no Relationships root".into()));
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

/// The relationship id of each `sheet` element of the workbook part, in order. Only
/// canonical markup is accepted, the kind calamine (last attribute ending in `:id`),
/// sheetparts (first local name after the last colon) and this check read the same way:
/// no element or attribute name in the part has more than one colon, and each sheet
/// carries only the unprefixed `name` and `sheetId`, optionally `state`, and exactly one
/// `<prefix>:id` whose prefix the workbook root declares as the relationships namespace.
fn sheet_ids(zip: &mut Archive, part: &str) -> Result<Vec<String>, Refusal> {
    let bytes = zip
        .read(part)
        .map_err(|e| (zip_reason(&e), format!("{part}: {e}")))?;
    let mut reader = Reader::from_reader(bytes.as_slice());
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let damaged = |what: String| (LossReason::Damaged, format!("{part}: {what}"));
    let colons = |name: &str| name.bytes().filter(|b| *b == b':').count();
    // Prefixes declared on the workbook root.
    let mut prefixes: Option<HashMap<String, String>> = None;
    loop {
        buf.clear();
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| damaged(e.to_string()))?;
        let e = match event {
            Event::Start(e) | Event::Empty(e) => e,
            Event::End(e) => {
                let name = e.name().as_ref().to_string();
                if colons(&name) > 1 {
                    return Err(damaged(format!("the element name {name} has two colons")));
                }
                continue;
            }
            Event::DocType(_) => return Err(damaged("a DOCTYPE is not allowed".into())),
            Event::Eof => break,
            _ => continue,
        };
        let name = e.name().as_ref().to_string();
        if colons(&name) > 1 {
            return Err(damaged(format!("the element name {name} has two colons")));
        }
        let mut attrs: Vec<(String, String)> = Vec::new();
        for a in e.attributes() {
            let a = a.map_err(|e| damaged(e.to_string()))?;
            let key = a.key.as_ref().to_string();
            if colons(&key) > 1 {
                return Err(damaged(format!("the attribute name {key} has two colons")));
            }
            attrs.push((key, a.value.to_string()));
        }
        if prefixes.is_none() {
            prefixes = Some(
                attrs
                    .iter()
                    .filter_map(|(k, v)| {
                        k.strip_prefix("xmlns:").map(|p| (p.to_string(), v.clone()))
                    })
                    .collect(),
            );
            continue;
        }
        if e.local_name().as_ref() != "sheet" {
            continue;
        }
        let Some(declared) = prefixes.as_ref() else {
            continue;
        };
        // Local names by the first and by the last colon (the same once no name has two
        // colons; both are kept so the rule does not rest on that).
        let mut by_first = HashSet::new();
        let mut by_last = HashSet::new();
        let mut id = None;
        for (key, value) in &attrs {
            let first = key.split_once(':').map_or(key.as_str(), |(_, l)| l);
            let last = key.rsplit_once(':').map_or(key.as_str(), |(_, l)| l);
            if !by_first.insert(first.to_string()) || !by_last.insert(last.to_string()) {
                return Err(damaged(format!("a sheet has two attributes named {last}")));
            }
            match key.split_once(':') {
                None if key == "name" && has_control(value) => {
                    return Err(damaged(format!(
                        "the sheet name {value:?} holds a control character"
                    )));
                }
                None if matches!(key.as_str(), "name" | "sheetId" | "state") => {}
                Some((p, "id")) if REL_NS.contains(&declared.get(p).map_or("", String::as_str)) => {
                    id = Some(value.clone());
                }
                Some((_, "id")) => {
                    return Err(damaged(format!(
                        "the sheet relationship id {key} is not in the relationships namespace"
                    )));
                }
                _ => return Err(damaged(format!("a sheet has the attribute {key}"))),
            }
        }
        if !by_first.contains("name") || !by_first.contains("sheetId") {
            return Err(damaged("a sheet lacks its name or sheetId".into()));
        }
        out.push(id.ok_or_else(|| damaged("a sheet without a relationship id".into()))?);
    }
    Ok(out)
}

/// A control character (U+0000 to U+001F, U+007F) Excel does not allow in a sheet name:
/// literal, or as a decimal or hexadecimal character reference in a raw attribute value.
pub(crate) fn has_control(raw: &str) -> bool {
    let control = |c: u32| c < 0x20 || c == 0x7f;
    if raw.chars().any(|c| control(c as u32)) {
        return true;
    }
    raw.split("&#").skip(1).any(|rest| {
        let Some((num, _)) = rest.split_once(';') else {
            return false;
        };
        let value = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16),
            None => num.parse::<u32>(),
        };
        value.is_ok_and(control)
    })
}

/// A target both calamine and sheetparts read the same way: no `.`, `..` or empty
/// segments, no backslash, no escaped characters.
pub(crate) fn clean(target: &str) -> bool {
    let path = target.strip_prefix('/').unwrap_or(target);
    !path.is_empty()
        && !target.contains(['\\', '&'])
        && path
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..")
}

/// The part a target names, as calamine joins it (absolute, or after `dir`).
pub(crate) fn join(dir: &str, target: &str) -> String {
    match target.strip_prefix('/') {
        Some(abs) => abs.to_string(),
        None => format!("{dir}{target}"),
    }
}

/// The relationships of every `.rels` part, each checked to be canonical.
pub(crate) fn all_rels(zip: &mut Archive) -> Result<HashMap<String, Vec<Rel>>, Refusal> {
    let names: Vec<String> = zip
        .names()
        .iter()
        .filter(|n| n.ends_with(".rels"))
        .map(|n| n.to_string())
        .collect();
    let mut parsed = HashMap::new();
    for name in names {
        let list = rels(zip, &name)?;
        parsed.insert(name, list);
    }
    Ok(parsed)
}

/// The one officeDocument relationship of `_rels/.rels`, internal and with a plain target.
pub(crate) fn office_document(parsed: &HashMap<String, Vec<Rel>>) -> Result<&Rel, Refusal> {
    let damaged = |what: String| (LossReason::Damaged, what);
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
    Ok(doc)
}

/// Refuses a package that calamine and sheetparts could read in two ways: duplicate
/// relationship ids in any `.rels` part, not exactly one officeDocument relationship,
/// unclean or external workbook and sheet targets, parts that are not there, and sheet
/// names that repeat (case-insensitive). Returns the part calamine reads for each sheet, in
/// the order of `wb.sheets`; each equals the part sheetparts resolved.
pub(crate) fn check_structure(
    zip: &mut Archive,
    wb: &WorkbookParts,
) -> Result<Vec<String>, Refusal> {
    let damaged = |what: String| (LossReason::Damaged, what);
    let names: Vec<String> = zip.names().iter().map(|n| n.to_string()).collect();
    let exists: HashSet<&str> = names.iter().map(String::as_str).collect();
    let parsed = all_rels(zip)?;

    let doc = office_document(&parsed)?;
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

    // Every sheet's relationship, by the id the workbook gives it and whatever its type
    // (sheetparts follows the id, calamine the last segment of the type): the part calamine
    // joins must be the part sheetparts resolved.
    let wb_rels = format!("{dir}_rels/workbook.xml.rels");
    let by_id: HashMap<&str, &Rel> = parsed
        .get(&wb_rels)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .map(|r| (r.id.as_str(), r))
        .collect();
    let ids = sheet_ids(zip, &part)?;
    if ids.len() != wb.sheets.len() {
        return Err(damaged(format!(
            "{part} lists {} sheets, {} were read",
            ids.len(),
            wb.sheets.len()
        )));
    }
    let mut calamine_parts = Vec::with_capacity(ids.len());
    for (id, s) in ids.iter().zip(&wb.sheets) {
        let Some(r) = by_id.get(id.as_str()) else {
            return Err(damaged(format!(
                "sheet {:?} names the relationship {id:?}, which is not there",
                s.name
            )));
        };
        if r.external || !clean(&r.target) {
            return Err(damaged(format!(
                "the target {:?} of sheet {:?} is external or not a plain path",
                r.target, s.name
            )));
        }
        let p = join(&dir, &r.target);
        if p != s.part || !exists.contains(p.as_str()) {
            return Err(damaged(format!(
                "the part {p} of sheet {:?} is not in the package",
                s.name
            )));
        }
        calamine_parts.push(p);
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
    Ok(calamine_parts)
}

/// Refuses a workbook whose sheet names, as calamine read them, are not exactly the names
/// Rezzer read (sheetparts, or the BIFF12 walker) in the same order, or repeat, or hold a
/// control character. calamine finds a sheet by its name.
pub(crate) fn check_names(calamine: &[String], ours: &[&str]) -> Result<(), Refusal> {
    let theirs: Vec<&str> = calamine.iter().map(String::as_str).collect();
    if ours != theirs {
        return Err((
            LossReason::Damaged,
            format!("the sheet names are read two ways: {ours:?} and {theirs:?}"),
        ));
    }
    if let Some(bad) = theirs.iter().find(|n| has_control(n)) {
        return Err((
            LossReason::Damaged,
            format!("the sheet name {bad:?} holds a control character"),
        ));
    }
    let mut seen = HashSet::new();
    if let Some(dup) = theirs.iter().find(|n| !seen.insert(**n)) {
        return Err((
            LossReason::Damaged,
            format!("the sheet name {dup:?} is used twice"),
        ));
    }
    Ok(())
}
