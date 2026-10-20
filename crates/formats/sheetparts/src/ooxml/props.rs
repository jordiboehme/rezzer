// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::BTreeMap;

use super::{Ctx, Rel};
use crate::PropValue;
use crate::xml::Ev;

/// Document properties: core (title, author, dates), app (application, company) and the
/// custom properties with their types.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DocProps {
    /// Core properties by local name, for example `creator`, `created`, `title`.
    pub core: BTreeMap<String, String>,
    /// Extended (app) properties by name, for example `Application`, `Company`.
    pub app: BTreeMap<String, String>,
    /// Custom properties in file order.
    pub custom: Vec<(String, PropValue)>,
}

/// Reads the document properties of any OOXML package (also XLSB, whose workbook parts are
/// binary but whose properties are XML): the parts `_rels/.rels` names, or the usual
/// `docProps/` parts. Problems come back as issues.
pub fn read_doc_props(
    zip: &mut zipguard::Archive<'_>,
    limits: &crate::Limits,
    stop: &dyn Fn() -> bool,
) -> (DocProps, Vec<crate::Issue>) {
    let mut ctx = Ctx {
        zip,
        limits,
        stop,
        issues: Vec::new(),
    };
    let root_rels = ctx.rels("");
    let props = read_props(&mut ctx, &root_rels);
    (props, ctx.issues)
}

pub(super) fn read_props(ctx: &mut Ctx<'_, '_>, root_rels: &[Rel]) -> DocProps {
    let find = |suffix: &str, default: &str| -> String {
        root_rels
            .iter()
            .find(|r| r.is(suffix) && !r.external)
            .map_or_else(|| default.to_string(), |r| r.target.clone())
    };
    let core = find("/core-properties", "docProps/core.xml");
    let app = find("/extended-properties", "docProps/app.xml");
    let custom = find("/custom-properties", "docProps/custom.xml");
    DocProps {
        core: leaf_children(ctx, &core),
        app: leaf_children(ctx, &app),
        custom: read_custom(ctx, &custom),
    }
}

/// The text of every leaf element directly under the root, by element name.
fn leaf_children(ctx: &mut Ctx<'_, '_>, part: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut key: Option<String> = None;
    let mut text = String::new();
    let mut nested = false;
    ctx.scan(part, false, &mut |ev, depth| match ev {
        Ev::Start(e) if depth == 2 => {
            key = Some(e.name.clone());
            text.clear();
            nested = false;
        }
        Ev::Start(_) => nested = true,
        Ev::Text(t) if key.is_some() => text.push_str(t),
        Ev::End(_) if depth == 2 => {
            if let Some(k) = key.take()
                && !nested
                && !text.trim().is_empty()
            {
                out.insert(k, text.trim().to_string());
            }
        }
        _ => {}
    });
    out
}

fn read_custom(ctx: &mut Ctx<'_, '_>, part: &str) -> Vec<(String, PropValue)> {
    let mut out = Vec::new();
    let mut name: Option<String> = None;
    let mut ty: Option<String> = None;
    let mut text = String::new();
    ctx.scan(part, false, &mut |ev, depth| match ev {
        Ev::Start(e) if depth == 2 && e.name == "property" => {
            name = Some(e.attr("name").unwrap_or("").to_string());
            ty = None;
        }
        Ev::Start(e) if depth == 3 && name.is_some() => {
            ty = Some(e.name.clone());
            text.clear();
        }
        Ev::Empty(e) if depth == 3 => {
            if let Some(n) = &name {
                out.push((n.clone(), typed(&e.name, "")));
            }
        }
        Ev::Text(t) if ty.is_some() => text.push_str(t),
        Ev::End(_) if depth == 3 => {
            if let (Some(n), Some(t)) = (&name, ty.take()) {
                out.push((n.clone(), typed(&t, &text)));
            }
        }
        Ev::End(_) if depth == 2 => name = None,
        _ => {}
    });
    out
}

fn typed(ty: &str, value: &str) -> PropValue {
    let other = || PropValue::Other {
        ty: ty.to_string(),
        value: value.to_string(),
    };
    match ty {
        "lpwstr" | "lpstr" | "bstr" => PropValue::Text(value.to_string()),
        "i1" | "i2" | "i4" | "i8" | "int" | "ui1" | "ui2" | "ui4" | "ui8" | "uint" => value
            .trim()
            .parse()
            .map_or_else(|_| other(), PropValue::Integer),
        "r4" | "r8" | "decimal" | "currency" => value
            .trim()
            .parse()
            .map_or_else(|_| other(), PropValue::Number),
        "bool" => match value.trim() {
            "true" | "1" => PropValue::Bool(true),
            "false" | "0" => PropValue::Bool(false),
            _ => other(),
        },
        "filetime" | "date" => PropValue::DateTime(value.trim().to_string()),
        _ => other(),
    }
}
