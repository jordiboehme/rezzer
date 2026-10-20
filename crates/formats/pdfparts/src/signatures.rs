// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Digital signatures: signature fields and the signatures in `/Perms`. They are described,
//! never verified.

use std::collections::HashSet;

use lopdf::{Dictionary, Object, ObjectId};

use crate::objects::{dict, get, get_date, get_dict, get_name, get_text, reference, resolve};
use crate::{Issue, IssueKind, Pdf, PdfDate};

/// One signature as the file describes it. It is not verified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sig {
    /// The full name of the signature field, `None` for a signature that only `/Perms` holds.
    pub field: Option<String>,
    pub kind: SigKind,
    /// `/Name`, the signer as the signing software wrote it.
    pub signer: Option<String>,
    pub reason: Option<String>,
    pub location: Option<String>,
    /// `/ContactInfo`.
    pub contact: Option<String>,
    /// `/SubFilter` without the slash, for example `adbe.pkcs7.detached`.
    pub sub_filter: Option<String>,
    /// `/M`, the signing time the signer claims.
    pub date: Option<PdfDate>,
    /// `/ByteRange` as written: offset and length pairs of the signed bytes.
    pub byte_range: Vec<u64>,
    /// The byte range starts at 0, ends at the end of the file and leaves out exactly one
    /// gap, which holds the `/Contents` value. Saves after the signature end later, so they
    /// make this false.
    pub covers_whole_file: bool,
}

/// Where a signature comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigKind {
    /// A signature field.
    Field,
    /// A usage rights signature (`/Perms /UR3` or `/UR`), which unlocks features in a viewer.
    UsageRights,
    /// A certification signature (`/Perms /DocMDP`), which says what may change after it.
    DocMdp,
}

const WHERE: &str = "signatures";

impl Sig {
    fn weight(&self) -> usize {
        let text = |s: &Option<String>| s.as_ref().map_or(0, String::len);
        text(&self.field)
            + text(&self.signer)
            + text(&self.reason)
            + text(&self.location)
            + text(&self.contact)
            + text(&self.sub_filter)
            + self.date.as_ref().map_or(0, |d| d.raw.len() * 2)
            + self.byte_range.len() * 8
    }
}

impl Pdf {
    /// Every signature: signature fields that hold a signature in tree order, then the
    /// signatures of `/Perms` that no field holds. Byte ranges are checked against the
    /// bytes the file was opened from. Read on the first call and kept.
    pub fn signatures(&self) -> &[Sig] {
        self.signatures.get_or_init(|| self.read_signatures())
    }

    fn read_signatures(&self) -> Vec<Sig> {
        let doc = &self.doc;
        let mut issues = Vec::new();
        let perms = self.catalog().and_then(|c| get_dict(doc, c, b"Perms"));
        let perm_kind = |key: &[u8]| match key {
            b"DocMDP" => Some(SigKind::DocMdp),
            b"UR3" | b"UR" => Some(SigKind::UsageRights),
            _ => None,
        };
        // The signature dictionaries of /Perms by object, to mark a certification field.
        let mut perm_ids: Vec<(ObjectId, SigKind)> = Vec::new();
        if let Some(p) = perms {
            for (k, v) in p.iter() {
                if let (Some(kind), Some(id)) = (perm_kind(k), reference(v)) {
                    perm_ids.push((id, kind));
                }
            }
        }
        let mut found: Vec<(Option<String>, SigKind, &Dictionary)> = Vec::new();
        let mut listed: HashSet<ObjectId> = HashSet::new();
        let mut budget = self.text_budget();
        self.walk_fields(&mut budget, &mut issues, |node, _, _| {
            if !node.terminal || !node.is_signature() {
                return Ok(());
            }
            let Some(v) = node.value() else {
                return Ok(());
            };
            let Some(sd) = dict(doc, v) else {
                return Ok(());
            };
            let mut kind = SigKind::Field;
            if let Some(id) = reference(v) {
                if !listed.insert(id) {
                    return Ok(());
                }
                if let Some((_, k)) = perm_ids.iter().find(|(p, _)| *p == id) {
                    kind = *k;
                }
            }
            found.push((Some(node.full_name.clone()), kind, sd));
            Ok(())
        });
        if let Some(p) = perms {
            for (k, v) in p.iter() {
                let Some(kind) = perm_kind(k) else {
                    continue;
                };
                if reference(v).is_some_and(|id| !listed.insert(id)) {
                    continue;
                }
                if let Some(sd) = dict(doc, v) {
                    found.push((None, kind, sd));
                }
            }
        }
        let mut out = Vec::new();
        for (field, kind, sd) in found {
            let sig = self.sig(field, kind, sd, &mut issues);
            if !budget.charge(sig.weight()) {
                issues.push(budget.limit_issue(WHERE, "signatures"));
                break;
            }
            out.push(sig);
        }
        self.note(issues);
        out
    }

    fn sig(
        &self,
        field: Option<String>,
        kind: SigKind,
        sd: &Dictionary,
        issues: &mut Vec<Issue>,
    ) -> Sig {
        let doc = &self.doc;
        let place = match &field {
            Some(f) => format!("field {f}"),
            None => WHERE.to_string(),
        };
        let mut byte_range = Vec::new();
        match get(doc, sd, b"ByteRange") {
            Some(Object::Array(a)) => {
                let values: Option<Vec<u64>> = a
                    .iter()
                    .take(self.limits.max_items)
                    .map(|o| match resolve(doc, o)? {
                        Object::Integer(i) => u64::try_from(*i).ok(),
                        _ => None,
                    })
                    .collect();
                match values {
                    Some(v) if v.len() % 2 == 0 => byte_range = v,
                    _ => issues.push(Issue::new(
                        &place,
                        IssueKind::Damaged,
                        "the signature's byte range is not a list of offset and length pairs",
                    )),
                }
            }
            Some(_) => issues.push(Issue::new(
                &place,
                IssueKind::Damaged,
                "the signature's byte range is not an array",
            )),
            None => {}
        }
        let contents = match get(doc, sd, b"Contents") {
            Some(Object::String(s, _)) => Some(s.len()),
            _ => None,
        };
        let covers_whole_file =
            contents.is_some_and(|len| covers_whole_file(self.original(), &byte_range, len));
        Sig {
            field,
            kind,
            signer: get_text(doc, sd, b"Name"),
            reason: get_text(doc, sd, b"Reason"),
            location: get_text(doc, sd, b"Location"),
            contact: get_text(doc, sd, b"ContactInfo"),
            sub_filter: get_name(doc, sd, b"SubFilter")
                .map(|n| String::from_utf8_lossy(n).into_owned()),
            date: get_date(doc, sd, b"M"),
            byte_range,
            covers_whole_file,
        }
    }
}

/// The range `[0, a, b, c]` ends at the end of `file`, and the gap from `a` to `b` is one
/// hex string that holds `contents` bytes, with or without its brackets.
fn covers_whole_file(file: &[u8], range: &[u64], contents: usize) -> bool {
    let &[0, a, b, c] = range else {
        return false;
    };
    let (Ok(a), Ok(b), Ok(c)) = (usize::try_from(a), usize::try_from(b), usize::try_from(c)) else {
        return false;
    };
    if a > b || b.checked_add(c) != Some(file.len()) {
        return false;
    }
    // The gap should hold the whole string with its brackets; some writers leave the
    // brackets in the signed part.
    let gap = &file[a..b];
    let inner = match gap.strip_prefix(b"<").and_then(|g| g.strip_suffix(b">")) {
        Some(inner) => inner,
        None if a > 0 && file[a - 1] == b'<' && file.get(b) == Some(&b'>') => gap,
        None => return false,
    };
    let mut digits = 0usize;
    for &byte in inner {
        if byte.is_ascii_hexdigit() {
            digits += 1;
        } else if !byte.is_ascii_whitespace() {
            return false;
        }
    }
    digits.div_ceil(2) == contents
}
