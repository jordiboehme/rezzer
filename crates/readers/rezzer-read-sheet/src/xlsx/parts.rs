// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Validation rules, comments, properties, VBA and embedded objects.

use super::*;

/// Validations and conditional formats, one rule per range they apply to.
pub(crate) fn rules(
    out: &mut Out,
    q: &str,
    validations: &[Validation],
    cond_formats: &[CondFormat],
    budget: &Budget,
) {
    let mut list: Vec<Rule> = Vec::new();
    for v in validations {
        let mut detail = v.kind.clone();
        if let Some(list) = &v.list {
            detail = format!("list: {}", list.join(", "));
        } else if v.kind != "list" && v.kind != "custom" && v.kind != "none" {
            detail.push(' ');
            detail.push_str(v.operator.as_deref().unwrap_or("between"));
        }
        let formulas: Vec<String> = [&v.formula1, &v.formula2]
            .into_iter()
            .flatten()
            .cloned()
            .collect();
        let message = match (&v.prompt, &v.error) {
            (Some(p), Some(e)) => Some(format!("prompt: {p}; error: {e}")),
            (Some(p), None) => Some(format!("prompt: {p}")),
            (None, Some(e)) => Some(format!("error: {e}")),
            (None, None) => None,
        };
        for range in v.range.split_whitespace() {
            list.push(Rule {
                kind: "validation".into(),
                anchor: Anchor(format!("{q}!{}", range.replace('$', ""))),
                detail: detail.clone(),
                formulas: formulas.clone(),
                message: message.clone(),
            });
        }
    }
    for cf in cond_formats {
        let mut detail = cf.kind.clone();
        if let Some(op) = &cf.operator {
            detail.push(' ');
            detail.push_str(op);
        }
        detail.push_str(&format!(" (priority {})", cf.priority));
        for range in cf.range.split_whitespace() {
            list.push(Rule {
                kind: "conditional-format".into(),
                anchor: Anchor(format!("{q}!{}", range.replace('$', ""))),
                detail: detail.clone(),
                formulas: cf.formulas.clone(),
                message: None,
            });
        }
    }
    let total = list.len();
    for (i, rule) in list.into_iter().enumerate() {
        if let Err(hit) = budget.add_elements(1) {
            out.loss(
                q,
                format!("{hit}: {} of {total} rules not read", total - i),
                LossReason::Limit,
                Severity::Warning,
            );
            break;
        }
        out.doc.facets.rules.push(rule);
    }
}

/// The inlines of a comment text, cut at the text limit (counted in `cut`).
pub(crate) fn comment_body(budget: &Budget, text: &str, cut: &mut u64) -> Vec<Inline> {
    if budget.charge_text(text.len()).is_err() {
        *cut += 1;
        let max = usize::try_from(budget.limits().max_text_bytes).unwrap_or(usize::MAX);
        crate::text_inlines(&grid::cut_text(text.to_string(), max))
    } else {
        crate::text_inlines(text)
    }
}

/// Notes and threaded comments at their cells; replies nested under their thread.
pub(super) fn annotations(out: &mut Out, q: &str, parts: &SheetParts, budget: &Budget) {
    let mut cut = 0u64;
    let mut body = |text: &str| comment_body(budget, text, &mut cut);
    let mut list = Vec::new();
    for n in &parts.notes {
        list.push(Annotation {
            kind: "note".into(),
            author: n.author.clone(),
            date: None,
            anchor: Some(Anchor(format!("{q}!{}", n.cell.replace('$', "")))),
            quoted: None,
            body: body(&n.text),
            replies: Vec::new(),
            resolved: None,
        });
    }
    let mut threads: Vec<Annotation> = Vec::new();
    let mut by_id: HashMap<&str, usize> = HashMap::new();
    let mut orphans = 0u64;
    for t in &parts.threads {
        let a = Annotation {
            kind: "comment".into(),
            author: t.person.clone(),
            date: t.date.clone(),
            anchor: Some(Anchor(format!("{q}!{}", t.cell.replace('$', "")))),
            quoted: None,
            body: body(&t.text),
            replies: Vec::new(),
            resolved: None,
        };
        let root = t.parent.as_deref().and_then(|p| by_id.get(p).copied());
        match root {
            Some(i) => threads[i].replies.push(a),
            None => {
                if t.parent.is_some() {
                    orphans += 1;
                }
                by_id.insert(t.id.as_str(), threads.len());
                threads.push(Annotation {
                    resolved: Some(t.done),
                    ..a
                });
            }
        }
    }
    list.extend(threads);
    add_annotations(out, q, list, budget, cut);
    if orphans > 0 {
        out.loss(
            q,
            format!("{orphans} comment replies point at a thread that is not there; shown as threads of their own"),
            LossReason::Damaged,
            Severity::Info,
        );
    }
}

/// Adds the annotations of a sheet within the element budget; `cut` texts were cut at the
/// text limit.
pub(crate) fn add_annotations(
    out: &mut Out,
    q: &str,
    list: Vec<Annotation>,
    budget: &Budget,
    cut: u64,
) {
    let total = list.len();
    for (i, a) in list.into_iter().enumerate() {
        let size = 1 + a.replies.len() as u64;
        if let Err(hit) = budget.add_elements(size) {
            out.loss(
                q,
                format!("{hit}: {} of {total} comments not read", total - i),
                LossReason::Limit,
                Severity::Warning,
            );
            break;
        }
        out.doc.facets.annotations.push(a);
    }
    if cut > 0 {
        out.loss(
            q,
            format!("{cut} comments longer than the text limit were cut"),
            LossReason::Limit,
            Severity::Warning,
        );
    }
}

/// Title, authors, dates, keywords, custom properties and, when the file has them, the
/// calculation settings and the date system.
pub(crate) fn properties(
    out: &mut Out,
    doc: &DocProps,
    calc: Option<&CalcSettings>,
    date1904: bool,
) {
    let props = &mut out.doc.properties;
    let core = &doc.core;
    props.title = core.get("title").cloned();
    props.description = core.get("description").cloned();
    if let Some(a) = core.get("creator") {
        props.authors.push(a.clone());
    }
    props.created = core
        .get("created")
        .map(|v| Value::DateTime { value: v.clone() });
    props.modified = core
        .get("modified")
        .map(|v| Value::DateTime { value: v.clone() });
    props.language = core.get("language").cloned();
    if let Some(k) = core.get("keywords") {
        props.keywords = k
            .split([',', ';'])
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(str::to_string)
            .collect();
    }
    let custom = &mut props.custom;
    let text = |s: &str| Value::Text {
        value: s.to_string(),
    };
    for (key, name) in [
        ("subject", "subject"),
        ("lastModifiedBy", "last_modified_by"),
        ("category", "category"),
    ] {
        if let Some(v) = core.get(key) {
            custom.insert(name.into(), text(v));
        }
    }
    for (key, name) in [("Application", "application"), ("Company", "company")] {
        if let Some(v) = doc.app.get(key) {
            custom.insert(name.into(), text(v));
        }
    }
    if let Some(calc) = calc {
        let mode = match calc.mode {
            CalcMode::Auto => "auto",
            CalcMode::Manual => "manual",
            CalcMode::AutoNoTable => "auto_no_table",
        };
        custom.insert("calc_mode".into(), text(mode));
        custom.insert(
            "iterative_calculation".into(),
            Value::Bool {
                value: calc.iterate,
            },
        );
        if calc.iterate {
            if let Some(n) = calc.iterate_count {
                custom.insert("iteration_count".into(), Value::number(f64::from(n), None));
            }
            if let Some(d) = calc.iterate_delta {
                custom.insert("iteration_delta".into(), Value::number(d, None));
            }
        }
    }
    if date1904 {
        custom.insert("date_system".into(), text("1904"));
    }
    // Custom document properties keep their names; Rezzer's own keys win a clash.
    for (name, value) in &doc.custom {
        let v = match value {
            PropValue::Text(s) => text(s),
            PropValue::Integer(i) => Value::number(*i as f64, None),
            PropValue::Number(n) => Value::number(*n, None),
            PropValue::Bool(b) => Value::Bool { value: *b },
            PropValue::DateTime(s) => Value::DateTime { value: s.clone() },
            PropValue::Other { value, .. } => text(value),
        };
        custom.entry(name.clone()).or_insert(v);
    }
}

/// VBA module names and the functions each one declares. The code is never run.
#[derive(Default)]
pub(crate) struct Vba {
    modules: Vec<String>,
    /// Uppercase function name to its module.
    functions: HashMap<String, String>,
}

pub(crate) fn vba_modules<RS, R>(out: &mut Out, cal: &mut R, part: &str) -> Vba
where
    RS: std::io::Read + std::io::Seek,
    R: calamine::Reader<RS>,
    R::Error: std::fmt::Display,
{
    let mut vba = Vba::default();
    let project = match contain(AssertUnwindSafe(|| cal.vba_project())) {
        Ok(Ok(Some(p))) => p,
        Ok(Ok(None)) => {
            out.loss(
                "document",
                "the package has a VBA project calamine does not find; module names not listed",
                LossReason::Unsupported,
                Severity::Info,
            );
            return vba;
        }
        Ok(Err(e)) => {
            out.loss(
                part,
                format!("VBA project not read: {e}"),
                LossReason::Damaged,
                Severity::Warning,
            );
            return vba;
        }
        Err(panic) => {
            out.loss(
                part,
                format!("VBA project not read: internal error: {panic}"),
                LossReason::Internal,
                Severity::Warning,
            );
            return vba;
        }
    };
    for name in project.get_module_names() {
        vba.modules.push(name.to_string());
        // The source is read as text to find the functions it declares; never run.
        if let Ok(code) = project.get_module(name) {
            for f in declared_functions(&code) {
                vba.functions
                    .entry(f.to_uppercase())
                    .or_insert_with(|| name.to_string());
            }
        }
    }
    vba
}

/// Names of the `Function` procedures a VBA module declares.
pub(crate) fn declared_functions(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in code.lines() {
        let mut words = line.split_whitespace().peekable();
        while let Some(w) = words.peek() {
            let lower = w.to_ascii_lowercase();
            if matches!(lower.as_str(), "public" | "private" | "friend" | "static") {
                words.next();
            } else {
                break;
            }
        }
        if words
            .next()
            .is_some_and(|w| w.eq_ignore_ascii_case("function"))
            && let Some(rest) = words.next()
        {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                out.push(name);
            }
        }
    }
    out
}

/// VBA modules, external workbooks and add-ins, and the user functions formulas call.
pub(crate) fn embedded(
    out: &mut Out,
    has_vba: bool,
    links: &[ExternalLink],
    vba: &Vba,
    formulas: &Formulas,
) {
    let list = &mut out.doc.facets.embedded;
    if has_vba && vba.modules.is_empty() {
        list.push(Embedded {
            text: None,
            kind: "macro".into(),
            name: "VBA project".into(),
            detail: Some("VBA project, never run".into()),
            target: None,
        });
    }
    for m in &vba.modules {
        list.push(Embedded {
            text: None,
            kind: "macro".into(),
            name: m.clone(),
            detail: Some("VBA module, never run".into()),
            target: None,
        });
    }
    for link in links {
        let lower = link.target.to_ascii_lowercase();
        let addin_file = [".xlam", ".xla", ".xll"].iter().any(|e| lower.ends_with(e));
        let kind = match link.kind.as_str() {
            "add-in" => "add-in",
            "workbook" if addin_file => "add-in",
            "workbook" => "external-workbook",
            other => other,
        };
        let name = link
            .target
            .rsplit(['/', '\\'])
            .next()
            .filter(|n| !n.is_empty())
            .unwrap_or(&link.target);
        list.push(Embedded {
            text: None,
            kind: kind.to_string(),
            name: name.to_string(),
            detail: Some(format!("[{}] in formulas, never opened", link.index)),
            target: Some(link.target.clone()),
        });
    }
    for (name, _first_use) in &formulas.user_functions {
        let (detail, target) = user_function_source(name, links, vba);
        list.push(Embedded {
            text: None,
            kind: "user-function".into(),
            name: name.clone(),
            detail: Some(detail),
            target,
        });
    }
}

/// Where a user function comes from: `VBA module` (target the module), the add-in or
/// workbook path of a `[n]!` prefix, or `unknown`.
fn user_function_source(name: &str, links: &[ExternalLink], vba: &Vba) -> (String, Option<String>) {
    if let Some(rest) = name.strip_prefix('[')
        && let Some((book, _)) = rest.split_once("]!")
    {
        if let Ok(index) = book.parse::<u32>() {
            return match links.iter().find(|l| l.index == index) {
                Some(l) => (l.target.clone(), None),
                None => ("unknown".into(), None),
            };
        }
        return (book.to_string(), None);
    }
    match vba.functions.get(&name.to_uppercase()) {
        Some(module) => ("VBA module".into(), Some(module.clone())),
        None => ("unknown".into(), None),
    }
}
