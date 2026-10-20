// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The parts of an OOXML workbook (xlsx, xlsm) that give cells their meaning.

mod props;
mod styles;
mod workbook;

pub use props::DocProps;
pub use styles::Styles;
pub use workbook::{
    CalcMode, CalcSettings, DefinedName, ExternalLink, Person, SheetInfo, SheetState,
    WorkbookParts, read_workbook,
};

use zipguard::{Archive, ZipError};

use crate::xml::{Elem, Ev, scan};
use crate::{Issue, IssueKind, Limits};

/// What every part reader needs: the archive, the bounds, the stop callback and the issue list.
pub(crate) struct Ctx<'z, 'a> {
    pub zip: &'z mut Archive<'a>,
    pub limits: &'z Limits,
    pub stop: &'z dyn Fn() -> bool,
    pub issues: Vec<Issue>,
}

impl Ctx<'_, '_> {
    /// Reads a part. A missing part is an issue only when `required`.
    pub fn bytes(&mut self, name: &str, required: bool) -> Option<Vec<u8>> {
        if !self.zip.contains(name) {
            if required {
                self.issues.push(Issue::new(
                    name,
                    IssueKind::Damaged,
                    "the part is not in the package",
                ));
            }
            return None;
        }
        match self.zip.read(name) {
            Ok(bytes) => Some(bytes),
            Err(e) => {
                let kind = match e {
                    ZipError::Limit { .. } => IssueKind::Limit,
                    ZipError::Encrypted(_) => IssueKind::Unsupported,
                    _ => IssueKind::Damaged,
                };
                self.issues.push(Issue::new(name, kind, e.to_string()));
                None
            }
        }
    }

    /// Reads a part and scans its XML. Returns false when there was nothing to scan or the scan
    /// ended early.
    pub fn scan(
        &mut self,
        name: &str,
        required: bool,
        on_event: &mut dyn FnMut(Ev<'_>, usize),
    ) -> bool {
        if (self.stop)() {
            self.issues
                .push(Issue::new(name, IssueKind::Limit, "stopped by the caller"));
            return false;
        }
        let Some(bytes) = self.bytes(name, required) else {
            return false;
        };
        scan(
            &bytes,
            name,
            self.limits,
            self.stop,
            &mut self.issues,
            on_event,
        )
    }

    /// The relationships of a part. A part without a relationships file has none.
    pub fn rels(&mut self, part: &str) -> Vec<Rel> {
        let rels_part = rels_part_for(part);
        let mut out = Vec::new();
        let base = part.to_string();
        self.scan(&rels_part, false, &mut |ev, _| {
            if let Ev::Start(e) | Ev::Empty(e) = ev
                && e.name == "Relationship"
            {
                out.push(Rel::from_elem(e, &base));
            }
        });
        out
    }
}

/// One relationship of a part.
#[derive(Debug, Clone)]
pub(crate) struct Rel {
    pub id: String,
    pub kind: String,
    /// The target as written for external targets, otherwise the resolved part name.
    pub target: String,
    pub external: bool,
}

impl Rel {
    fn from_elem(e: &Elem, base: &str) -> Rel {
        let external = e
            .attr("TargetMode")
            .is_some_and(|m| m.eq_ignore_ascii_case("External"));
        let raw = e.attr("Target").unwrap_or("");
        Rel {
            id: e.attr("Id").unwrap_or("").to_string(),
            kind: e.attr("Type").unwrap_or("").to_string(),
            target: if external {
                raw.to_string()
            } else {
                resolve(base, raw)
            },
            external,
        }
    }

    pub fn is(&self, suffix: &str) -> bool {
        self.kind.ends_with(suffix)
    }
}

/// `xl/workbook.xml` has its relationships in `xl/_rels/workbook.xml.rels`.
pub(crate) fn rels_part_for(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

/// Resolves a relationship target against the part it belongs to.
pub(crate) fn resolve(base_part: &str, target: &str) -> String {
    let target = target.replace('\\', "/");
    let mut segments: Vec<&str> = Vec::new();
    let joined;
    let path = if let Some(absolute) = target.strip_prefix('/') {
        absolute
    } else {
        let dir = base_part.rsplit_once('/').map_or("", |(d, _)| d);
        joined = if dir.is_empty() {
            target.clone()
        } else {
            format!("{dir}/{target}")
        };
        &joined
    };
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            s => segments.push(s),
        }
    }
    segments.join("/")
}
