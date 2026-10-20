// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use zipguard::Archive;

use super::props::{DocProps, read_props};
use super::styles::{Styles, read_styles};
use super::{Ctx, Rel};
use crate::xml::Ev;
use crate::{DefinedName, Issue, IssueKind, Limits};

/// Whether a sheet is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetState {
    Visible,
    Hidden,
    /// Hidden so that the user cannot unhide it in the program.
    VeryHidden,
}

/// One sheet of the workbook, in tab order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetInfo {
    pub name: String,
    pub state: SheetState,
    /// The part name in the package, resolved through the workbook relationships.
    pub part: String,
}

/// When the workbook recalculates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalcMode {
    Auto,
    Manual,
    AutoNoTable,
}

/// The calculation settings of the workbook.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalcSettings {
    pub mode: CalcMode,
    /// Whether circular references are iterated.
    pub iterate: bool,
    pub iterate_count: Option<u32>,
    pub iterate_delta: Option<f64>,
    pub full_calc_on_load: bool,
}

impl Default for CalcSettings {
    fn default() -> Self {
        CalcSettings {
            mode: CalcMode::Auto,
            iterate: false,
            iterate_count: None,
            iterate_delta: None,
            full_calc_on_load: false,
        }
    }
}

/// A link to another workbook, an add-in, a DDE source or an OLE object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalLink {
    /// The `n` of `[n]` in formulas: the 1-based position in the workbook.
    pub index: u32,
    /// The target as written (a file path or URL, `service|topic` for DDE, a program id for
    /// OLE without a target).
    pub target: String,
    /// `workbook`, `add-in`, `dde` or `ole`.
    pub kind: String,
}

/// A person who wrote a threaded comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub id: String,
    pub name: String,
}

/// Everything read from the workbook-level parts.
#[derive(Debug, Clone, Default)]
pub struct WorkbookParts {
    /// The workbook part in the package, for example `xl/workbook.xml`; empty when the
    /// package has none.
    pub workbook_part: String,
    pub sheets: Vec<SheetInfo>,
    pub names: Vec<DefinedName>,
    pub calc: CalcSettings,
    pub styles: Styles,
    pub props: DocProps,
    pub external_links: Vec<ExternalLink>,
    pub persons: Vec<Person>,
    /// The package carries a VBA project (it is never read or run).
    pub has_vba: bool,
    /// The workbook counts dates from 1904 (`workbookPr date1904`).
    pub date1904: bool,
    pub issues: Vec<Issue>,
}

/// Reads the workbook-level parts of an xlsx or xlsm package.
///
/// Problems end up in `issues`; whatever could be read is still returned. `stop` is asked
/// between parts and now and then inside a part.
pub fn read_workbook(zip: &mut Archive, limits: &Limits, stop: &dyn Fn() -> bool) -> WorkbookParts {
    let mut ctx = Ctx {
        zip,
        limits,
        stop,
        issues: Vec::new(),
    };
    let mut out = WorkbookParts::default();

    let root_rels = ctx.rels("");
    let workbook_part = root_rels
        .iter()
        .find(|r| r.is("/officeDocument") && !r.external)
        .map(|r| r.target.clone())
        .or_else(|| {
            ctx.zip
                .contains("xl/workbook.xml")
                .then(|| "xl/workbook.xml".to_string())
        });
    out.has_vba = ctx
        .zip
        .names()
        .iter()
        .any(|n| n.to_ascii_lowercase().ends_with("vbaproject.bin"));
    out.props = read_props(&mut ctx, &root_rels);

    let Some(workbook_part) = workbook_part else {
        ctx.issues.push(Issue::new(
            "_rels/.rels",
            IssueKind::Damaged,
            "the package has no workbook part",
        ));
        out.issues = ctx.issues;
        return out;
    };

    out.workbook_part = workbook_part.clone();
    let rels = ctx.rels(&workbook_part);
    let mut raw_names: Vec<(DefinedName, Option<usize>)> = Vec::new();
    let mut external_rids: Vec<String> = Vec::new();
    let mut current: Option<(DefinedName, Option<usize>)> = None;
    let mut text = String::new();
    let mut sheet_rids: Vec<String> = Vec::new();
    ctx.scan(&workbook_part, true, &mut |ev, _| match ev {
        Ev::Start(e) | Ev::Empty(e) => match e.name.as_str() {
            "sheet" => {
                out.sheets.push(SheetInfo {
                    name: e.attr("name").unwrap_or("").to_string(),
                    state: match e.attr("state") {
                        Some("hidden") => SheetState::Hidden,
                        Some("veryHidden") => SheetState::VeryHidden,
                        _ => SheetState::Visible,
                    },
                    part: String::new(),
                });
                sheet_rids.push(e.attr("id").unwrap_or("").to_string());
            }
            "definedName" => {
                let name = DefinedName {
                    name: e.attr("name").unwrap_or("").to_string(),
                    scope: None,
                    refers_to: String::new(),
                    hidden: e.flag("hidden"),
                    comment: e.attr("comment").map(str::to_string),
                };
                let local = e.attr("localSheetId").and_then(|v| v.parse::<usize>().ok());
                if matches!(ev, Ev::Start(_)) {
                    text.clear();
                    current = Some((name, local));
                } else {
                    raw_names.push((name, local));
                }
            }
            "calcPr" => {
                out.calc = CalcSettings {
                    mode: match e.attr("calcMode") {
                        Some("manual") => CalcMode::Manual,
                        Some("autoNoTable") => CalcMode::AutoNoTable,
                        _ => CalcMode::Auto,
                    },
                    iterate: e.flag("iterate"),
                    iterate_count: e.attr("iterateCount").and_then(|v| v.parse().ok()),
                    iterate_delta: e.attr("iterateDelta").and_then(|v| v.parse().ok()),
                    full_calc_on_load: e.flag("fullCalcOnLoad"),
                };
            }
            "workbookPr" => out.date1904 = e.flag("date1904"),
            "externalReference" => external_rids.push(e.attr("id").unwrap_or("").to_string()),
            _ => {}
        },
        Ev::Text(t) if current.is_some() => text.push_str(t),
        Ev::End("definedName") => {
            if let Some((mut name, local)) = current.take() {
                name.refers_to = text.trim().to_string();
                raw_names.push((name, local));
            }
        }
        _ => {}
    });

    // Sheet parts through the relationships.
    for (sheet, rid) in out.sheets.iter_mut().zip(&sheet_rids) {
        match rels.iter().find(|r| &r.id == rid && !r.external) {
            Some(rel) => {
                sheet.part = rel.target.clone();
                if !ctx.zip.contains(&rel.target) {
                    ctx.issues.push(Issue::new(
                        &workbook_part,
                        IssueKind::Damaged,
                        format!(
                            "sheet {} points at {}, which is not in the package",
                            sheet.name, rel.target
                        ),
                    ));
                }
            }
            None => ctx.issues.push(Issue::new(
                &workbook_part,
                IssueKind::Damaged,
                format!("sheet {} has no relationship {rid}", sheet.name),
            )),
        }
    }

    // Name scopes: localSheetId counts the sheets in file order.
    for (mut name, local) in raw_names {
        if let Some(index) = local {
            match out.sheets.get(index) {
                Some(sheet) => name.scope = Some(sheet.name.clone()),
                None => ctx.issues.push(Issue::new(
                    &workbook_part,
                    IssueKind::Damaged,
                    format!(
                        "name {} is local to sheet {index}, which does not exist",
                        name.name
                    ),
                )),
            }
        }
        out.names.push(name);
    }

    if out.sheets.is_empty() && ctx.issues.is_empty() {
        ctx.issues.push(Issue::new(
            &workbook_part,
            IssueKind::Damaged,
            "the workbook lists no sheets",
        ));
    }
    if rels.iter().any(|r| r.is("/vbaProject")) {
        out.has_vba = true;
    }

    // Styles.
    let styles_part = rels
        .iter()
        .find(|r| r.is("/styles") && !r.external)
        .map(|r| r.target.clone())
        .or_else(|| {
            ctx.zip
                .contains("xl/styles.xml")
                .then(|| "xl/styles.xml".to_string())
        });
    if let Some(part) = styles_part {
        out.styles = read_styles(&mut ctx, &part);
    }

    // External links, numbered by their position in externalReferences.
    for (i, rid) in external_rids.iter().enumerate() {
        let link = read_external_link(&mut ctx, &rels, rid, (i + 1) as u32);
        out.external_links.push(link);
    }

    // People behind threaded comments.
    let person_part = rels
        .iter()
        .find(|r| r.is("/person") && !r.external)
        .map(|r| r.target.clone())
        .or_else(|| {
            ctx.zip
                .contains("xl/persons/person.xml")
                .then(|| "xl/persons/person.xml".to_string())
        });
    if let Some(part) = person_part {
        ctx.scan(&part, true, &mut |ev, _| {
            if let Ev::Start(e) | Ev::Empty(e) = ev
                && e.name == "person"
            {
                out.persons.push(Person {
                    id: e.attr("id").unwrap_or("").to_string(),
                    name: e.attr("displayName").unwrap_or("").to_string(),
                });
            }
        });
    }

    out.issues = ctx.issues;
    out
}

fn read_external_link(ctx: &mut Ctx<'_, '_>, rels: &[Rel], rid: &str, index: u32) -> ExternalLink {
    let mut link = ExternalLink {
        index,
        target: String::new(),
        kind: "workbook".to_string(),
    };
    let Some(rel) = rels.iter().find(|r| r.id == rid && !r.external) else {
        ctx.issues.push(Issue::new(
            "xl/workbook.xml",
            IssueKind::Damaged,
            format!("external reference {index} has no relationship {rid}"),
        ));
        return link;
    };
    let part = rel.target.clone();
    let part_rels = ctx.rels(&part);
    let rel_of = |id: Option<&str>| {
        id.and_then(|id| part_rels.iter().find(|r| r.id == id))
            .cloned()
    };
    let mut found: Option<(String, Option<String>, Option<String>)> = None;
    ctx.scan(&part, true, &mut |ev, _| {
        if let Ev::Start(e) | Ev::Empty(e) = ev
            && found.is_none()
        {
            match e.name.as_str() {
                "externalBook" => {
                    found = Some(("workbook".into(), e.attr("id").map(str::to_string), None))
                }
                "oleLink" => {
                    found = Some((
                        "ole".into(),
                        e.attr("id").map(str::to_string),
                        e.attr("progId").map(str::to_string),
                    ))
                }
                "ddeLink" => {
                    let target = format!(
                        "{}|{}",
                        e.attr("ddeService").unwrap_or(""),
                        e.attr("ddeTopic").unwrap_or("")
                    );
                    found = Some(("dde".into(), None, Some(target)));
                }
                _ => {}
            }
        }
    });
    let Some((kind, link_rid, extra)) = found else {
        return link;
    };
    link.kind = kind.clone();
    match kind.as_str() {
        "dde" => link.target = extra.unwrap_or_default(),
        _ => match rel_of(link_rid.as_deref()) {
            Some(target) => {
                if kind == "workbook" && target.kind.contains("/xlExternalLinkPath/") {
                    link.kind = "add-in".to_string();
                }
                link.target = target.target;
            }
            None => {
                link.target = extra.unwrap_or_default();
                if link.target.is_empty() {
                    ctx.issues.push(Issue::new(
                        &part,
                        IssueKind::Damaged,
                        "the external link has no target relationship",
                    ));
                }
            }
        },
    }
    link
}
