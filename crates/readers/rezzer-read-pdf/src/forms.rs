// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The form logic in the model: each field with its role, value, format in words and place in
//! the calculation order; each calculation as a `pdf-js` formula with the fields it reads; the
//! graph of which field reads which; every script and action of the file, listed with its
//! text; the signatures with the saves after them; and the facts about all of these. Nothing
//! is run, followed or verified.
//!
//! Every copy is charged to the form logic's own share ([`Kept`]): signatures first, as they
//! are few and small, then the fields with their calculations, the graph and the scripts. A
//! field that does not fit ends the fields with one loss; a script whose text does not fit is
//! kept without the text, and one loss says how many.

use std::collections::HashMap;

use pdfparts::{FieldKind, Form, Issue, IssueKind, Pdf, Script, Sig, SigKind};
use rezzer_model::{
    Anchor, Edge, Embedded, FieldAction, FormField, Formula, Graph, LossReason, Node, Role,
    Severity, Signature, Value,
};

use crate::Out;
use crate::objects::{Kept, iso, quote};

/// How many characters of a format script stand for the format when there are no words for it.
const FORMAT_CHARS: usize = 80;

/// The note of a calculation that builds field names at run time.
const DYNAMIC: &str = "reads unknown fields, see script";

/// Reads the form logic of `pdf` into `out`. `ends` are where the saves of the file end,
/// oldest first.
pub(crate) fn read(pdf: &Pdf, ends: &[u64], kept: &Kept<'_>, out: &mut Out) {
    let form = pdf.form();
    let scripts = pdf.scripts();
    let sigs = pdf.signatures();
    // A list pdfparts cut short does not give the file's count, nor its absence of something.
    let issues = pdf.issues();
    let cut = |location: &str| {
        issues
            .iter()
            .any(|i: &Issue| i.location == location && i.kind == IssueKind::Limit)
    };
    let (form_cut, scripts_cut, sigs_cut) = (cut("form"), cut("scripts"), cut("signatures"));
    drop(issues);
    facts(
        form,
        scripts,
        sigs,
        form_cut,
        scripts_cut || form_cut,
        sigs_cut,
        out,
    );
    if form.is_some_and(|f| f.xfa) {
        out.loss(
            "form",
            "XFA form data present, not read",
            LossReason::NotImplemented,
            Severity::Warning,
        );
    }
    signatures(sigs, ends, kept, out);
    if let Some(form) = form {
        fields(form, kept, out);
    }
    list_scripts(scripts, kept, out);
}

/// The facts about the form, scripts and signatures; a fact a cut list cannot tell is left
/// out.
fn facts(
    form: Option<&Form>,
    scripts: &[Script],
    sigs: &[Sig],
    form_cut: bool,
    scripts_cut: bool,
    sigs_cut: bool,
    out: &mut Out,
) {
    let facts = &mut out.doc.properties.facts;
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    if let Some(form) = form
        && !form.fields.is_empty()
        && !form_cut
    {
        facts.form_fields = Some(count(form.fields.len()));
    }
    let javascript = scripts.iter().any(|s| s.kind == "javascript");
    if javascript || !scripts_cut {
        facts.has_javascript = Some(javascript);
    }
    if !sigs.is_empty() || !sigs_cut {
        facts.signed = Some(!sigs.is_empty());
    }
    if !sigs.is_empty() && !sigs_cut {
        facts.signatures = Some(count(sigs.len()));
    }
}

/// The anchor of the field `name`.
fn anchor(name: &str) -> Anchor {
    Anchor(format!("field:{name}"))
}

/// Where the signed bytes end: the end of the last range of `/ByteRange`, `None` when the
/// pairs do not add up.
fn signed_end(range: &[u64]) -> Option<u64> {
    if range.is_empty() || !range.len().is_multiple_of(2) {
        return None;
    }
    range
        .chunks(2)
        .map(|p| p[0].checked_add(p[1]))
        .try_fold(0u64, |end, e| e.map(|e| end.max(e)))
}

/// One model signature per signature, with the saves that end after its signed bytes.
fn signatures(sigs: &[Sig], ends: &[u64], kept: &Kept<'_>, out: &mut Out) {
    for (k, s) in sigs.iter().enumerate() {
        let later =
            signed_end(&s.byte_range).map_or(0, |end| ends.iter().filter(|&&e| e > end).count());
        let model = Signature {
            anchor: s.field.as_deref().map(anchor),
            kind: match s.kind {
                SigKind::Field => "field",
                SigKind::UsageRights => "usage-rights",
                SigKind::DocMdp => "doc-mdp",
            }
            .into(),
            signer: s.signer.as_deref().map(str::to_string),
            date: iso(s.date.as_ref()).map(str::to_string),
            reason: s.reason.as_deref().map(str::to_string),
            location: s.location.as_deref().map(str::to_string),
            sub_filter: s.sub_filter.as_deref().map(str::to_string),
            covers_whole_file: s.covers_whole_file,
            later_revisions: u32::try_from(later).unwrap_or(u32::MAX),
        };
        let texts = [
            model.anchor.as_ref().map_or("", |a| a.0.as_str()),
            &model.kind,
            model.signer.as_deref().unwrap_or_default(),
            model.date.as_deref().unwrap_or_default(),
            model.reason.as_deref().unwrap_or_default(),
            model.location.as_deref().unwrap_or_default(),
            model.sub_filter.as_deref().unwrap_or_default(),
        ];
        if let Err(why) = kept.take(1, &texts) {
            let left = sigs.len() - k;
            out.loss(
                "signatures",
                format!(
                    "{left} of {} signatures {} not kept, from signature {} on: {why}",
                    sigs.len(),
                    if left == 1 { "is" } else { "are" },
                    k + 1
                ),
                LossReason::Limit,
                Severity::Warning,
            );
            return;
        }
        out.doc.facets.signatures.push(model);
    }
}

/// The model's word for a field type.
fn kind_word(kind: &FieldKind) -> &'static str {
    match kind {
        FieldKind::Text => "text",
        FieldKind::Checkbox => "checkbox",
        FieldKind::Radio => "radio",
        FieldKind::PushButton => "button",
        FieldKind::Combo => "combo",
        FieldKind::List => "list",
        FieldKind::Signature => "signature",
        FieldKind::Unknown(_) => "unknown",
    }
}

/// The start of a script on one line: its words joined by single spaces, cut to
/// [`FORMAT_CHARS`] characters, the last three `...`, when it is longer. Only that much of the
/// text is read.
fn short(script: &str) -> String {
    let mut out = String::new();
    let mut chars = 0;
    for word in script.split_whitespace() {
        if chars > FORMAT_CHARS {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
            chars += 1;
        }
        for c in word
            .chars()
            .take(FORMAT_CHARS + 1 - chars.min(FORMAT_CHARS))
        {
            out.push(c);
            chars += 1;
        }
    }
    if chars <= FORMAT_CHARS {
        return out;
    }
    let cut: String = out.chars().take(FORMAT_CHARS - 3).collect();
    format!("{}...", cut.trim_end())
}

/// What the scripts of the form that were not kept are: how many, and the first.
#[derive(Default)]
struct Dropped {
    count: usize,
    first: Option<(String, String)>,
    why: String,
}

impl Dropped {
    fn add(&mut self, field: &str, trigger: &str, why: String) {
        self.count += 1;
        if self.first.is_none() {
            self.first = Some((quote(field), trigger.to_string()));
            self.why = why;
        }
    }
}

/// The fields with their calculations, then the graph of which field reads which.
fn fields(form: &Form, kept: &Kept<'_>, out: &mut Out) {
    let mut calc: HashMap<&str, u32> = HashMap::new();
    for (k, name) in form.calc_order.iter().enumerate() {
        calc.entry(name.as_str())
            .or_insert(u32::try_from(k + 1).unwrap_or(u32::MAX));
    }
    let total = form.fields.len();
    let mut dropped = Dropped::default();
    // The kept fields' names in model order, each with the names its calculation reads.
    let mut kept_fields: Vec<(&str, Vec<String>)> = Vec::new();
    for (k, f) in form.fields.iter().enumerate() {
        let name = f.full_name.as_str();
        let order = calc.get(name).copied();
        let calculates = f.actions.iter().any(|(event, _)| event == "calculate");
        let role = if calculates {
            Role::Result
        } else if f.flags & 1 != 0 {
            Role::Constant
        } else {
            Role::Input
        };
        let format = f
            .actions
            .iter()
            .find(|(event, _)| event == "format")
            .map(|(_, text)| pdfparts::script::describe(text).unwrap_or_else(|| short(text)));
        // Charged from what pdfparts holds, before anything is copied.
        let kind = kind_word(&f.kind);
        let mut texts: Vec<&str> = vec![
            "field:",
            name,
            name,
            f.alt_name.as_deref().unwrap_or_default(),
            kind,
            f.value.as_deref().unwrap_or_default(),
            format.as_deref().unwrap_or_default(),
        ];
        texts.extend(f.flag_words.iter().copied());
        texts.extend(f.options.iter().map(|o| &**o));
        let elements = 1 + f.flag_words.len() + f.options.len();
        let pages = 4 * f.pages.len() as u64;
        if let Err(why) = kept.take_with(elements as u64, &texts, pages) {
            let left = total - k;
            out.loss(
                "form",
                format!(
                    "{left} of {total} form fields {} not kept, from field {} on: {why}",
                    if left == 1 { "is" } else { "are" },
                    quote(name)
                ),
                LossReason::Limit,
                Severity::Warning,
            );
            break;
        }
        drop(texts);
        let mut field = FormField {
            anchor: anchor(name),
            name: name.to_string(),
            label: f.alt_name.as_deref().map(str::to_string),
            kind: kind.to_string(),
            value: f.value.as_deref().map(|v| Value::Text {
                value: v.to_string(),
            }),
            role: Some(role),
            format,
            calc_order: order,
            actions: Vec::new(),
            flags: f.flag_words.iter().map(|w| (*w).to_string()).collect(),
            options: f.options.iter().map(|o| o.to_string()).collect(),
            pages: f.pages.clone(),
        };
        let mut reads: Vec<String> = Vec::new();
        for (event, text) in &f.actions {
            let fits = kept
                .budget()
                .charge_text(text.len())
                .map_err(|hit| hit.to_string());
            let words = fits
                .is_ok()
                .then(|| pdfparts::script::describe(text))
                .flatten();
            let fits = fits.and_then(|()| {
                kept.take(
                    1,
                    &[
                        event.as_str(),
                        &**text,
                        words.as_deref().unwrap_or_default(),
                    ],
                )
            });
            if let Err(why) = fits {
                dropped.add(name, event, why);
                continue;
            }
            if event == "calculate" {
                let refs = pdfparts::script::field_refs(text);
                let formula = Formula {
                    anchor: anchor(name),
                    expression: text.to_string(),
                    dialect: "pdf-js".into(),
                    references: refs.names.iter().map(|n| anchor(n)).collect(),
                    cached: None,
                    role: Some(Role::Result),
                    pattern: None,
                    latex: None,
                    note: refs.dynamic.then(|| DYNAMIC.to_string()),
                    kind: None,
                    range: None,
                    original: None,
                };
                let note = formula.note.as_deref().unwrap_or_default();
                let mut texts: Vec<&str> = vec![&formula.anchor.0, &**text, &formula.dialect, note];
                texts.extend(formula.references.iter().map(|a| a.0.as_str()));
                if let Err(why) = kept.take(1 + formula.references.len() as u64, &texts) {
                    dropped.add(name, event, why);
                } else {
                    out.doc.facets.formulas.push(formula);
                    reads.extend(refs.names);
                }
            }
            field.actions.push(FieldAction {
                trigger: event.clone(),
                script: text.to_string(),
                words,
            });
        }
        out.doc.facets.fields.push(field);
        kept_fields.push((name, reads));
    }
    if let Some((field, trigger)) = dropped.first {
        let n = dropped.count;
        out.loss(
            "form",
            format!(
                "{n} field {} not kept, the first the {trigger} script of field {field}: {}",
                if n == 1 { "script is" } else { "scripts are" },
                dropped.why
            ),
            LossReason::Limit,
            Severity::Warning,
        );
    }
    graph(&kept_fields, kept, out);
}

/// The graph `form` of which kept field reads which, when one reads another: a node for each
/// field that reads or is read, an edge `reads` from the reader. Names that are no kept field
/// stay in the formula's references only.
fn graph(kept_fields: &[(&str, Vec<String>)], kept: &Kept<'_>, out: &mut Out) {
    let index: HashMap<&str, usize> = kept_fields
        .iter()
        .enumerate()
        .rev()
        .map(|(i, (name, _))| (*name, i))
        .collect();
    let mut edges: Vec<(usize, usize)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (i, (_, reads)) in kept_fields.iter().enumerate() {
        for r in reads {
            if let Some(&j) = index.get(r.as_str())
                && seen.insert((i, j))
            {
                edges.push((i, j));
            }
        }
    }
    if edges.is_empty() {
        return;
    }
    let mut in_graph = vec![false; kept_fields.len()];
    for &(i, j) in &edges {
        in_graph[i] = true;
        in_graph[j] = true;
    }
    let mut g = Graph {
        id: "form".into(),
        kind: "dependency".into(),
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    let id = |i: usize| format!("f{}", i + 1);
    // The kept fields are the last ones of the model's list.
    let fields = &out.doc.facets.fields[out.doc.facets.fields.len() - kept_fields.len()..];
    let mut cut: Option<String> = None;
    for (i, _) in in_graph.iter().enumerate().filter(|(_, on)| **on) {
        let f = &fields[i];
        let role = f.role.map_or("input", role_word);
        let label = match f.calc_order {
            Some(n) => format!("{} ({role}, calc {n})", f.name),
            None => format!("{} ({role})", f.name),
        };
        let node = Node {
            id: id(i),
            label,
            kind: None,
            anchor: Some(f.anchor.clone()),
        };
        if let Err(why) = kept.take(1, &[&node.id, &node.label, &f.anchor.0]) {
            cut = Some(why);
            break;
        }
        g.nodes.push(node);
    }
    let shown = g.nodes.len();
    if cut.is_none() {
        for &(i, j) in &edges {
            let (from, to) = (id(i), id(j));
            if let Err(why) = kept.take(1, &[&from, &to, "reads"]) {
                cut = Some(why);
                break;
            }
            g.edges.push(Edge {
                from,
                to,
                kind: "reads".into(),
                anchor: None,
            });
        }
    }
    if let Some(why) = cut {
        let nodes = in_graph.iter().filter(|on| **on).count();
        out.loss(
            "form",
            format!(
                "the form graph keeps {shown} of {nodes} fields and {} of {} reads: {why}",
                g.edges.len(),
                edges.len()
            ),
            LossReason::Limit,
            Severity::Warning,
        );
    }
    out.doc.facets.graphs.push(g);
}

fn role_word(role: Role) -> &'static str {
    match role {
        Role::Input => "input",
        Role::Constant => "constant",
        Role::Result => "result",
        Role::Intermediate => "intermediate",
    }
}

/// Every script and action of the file as an embedded `script`, with its text: the trigger
/// as the name, the action's kind and the script's name as the detail. Link actions without a
/// script are left out: the links list names them. A text that does not fit is left out of
/// its entry; an entry that does not fit ends the list.
fn list_scripts(scripts: &[Script], kept: &Kept<'_>, out: &mut Out) {
    let listed: Vec<&Script> = scripts
        .iter()
        .filter(|s| s.text.is_some() || !s.trigger.starts_with("link page "))
        .collect();
    let total = listed.len();
    let mut dropped = 0usize;
    let mut first_dropped: Option<(String, String)> = None;
    for (k, s) in listed.iter().enumerate() {
        let detail = match &s.name {
            Some(name) => format!("{}, {name}", s.kind),
            None => s.kind.clone(),
        };
        let target = s.target.as_deref().unwrap_or_default();
        let base = ["script", s.trigger.as_str(), detail.as_str(), target];
        let mut text = s.text.as_deref();
        if let Some(t) = text {
            let mut texts = base.to_vec();
            texts.push(t);
            let fits = kept
                .budget()
                .charge_text(t.len())
                .map_err(|hit| hit.to_string())
                .and_then(|()| kept.take(1, &texts));
            match fits {
                Ok(()) => {}
                Err(why) => {
                    dropped += 1;
                    first_dropped.get_or_insert_with(|| (s.trigger.clone(), why));
                    text = None;
                }
            }
        }
        if text.is_none()
            && let Err(why) = kept.take(1, &base)
        {
            let left = total - k;
            out.loss(
                "scripts",
                format!(
                    "{left} of {total} scripts and actions {} not listed, from the {} one on: \
                     {why}",
                    if left == 1 { "is" } else { "are" },
                    quote(&s.trigger)
                ),
                LossReason::Limit,
                Severity::Warning,
            );
            break;
        }
        out.doc.facets.embedded.push(Embedded {
            kind: "script".into(),
            name: s.trigger.clone(),
            detail: Some(detail),
            target: s.target.as_deref().map(str::to_string),
            text: text.map(str::to_string),
        });
    }
    if let Some((trigger, why)) = first_dropped {
        out.loss(
            "scripts",
            format!(
                "the text of {dropped} {} not kept, the first that of {}: {why}",
                if dropped == 1 {
                    "script is"
                } else {
                    "scripts are"
                },
                quote(&trigger)
            ),
            LossReason::Limit,
            Severity::Warning,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_scripts_keep_their_words() {
        assert_eq!(short("  a\n\tb  c "), "a b c");
        let long = "word ".repeat(40);
        let cut = short(&long);
        assert!(
            cut.ends_with("...") && cut.chars().count() <= FORMAT_CHARS,
            "{cut}"
        );
        assert!(cut.starts_with("word word "), "{cut}");
        let one = "é".repeat(200);
        assert_eq!(short(&one).chars().count(), FORMAT_CHARS);
    }

    #[test]
    fn signed_end_of_byte_ranges() {
        assert_eq!(signed_end(&[0, 644, 8836, 1416]), Some(10252));
        assert_eq!(signed_end(&[0, 10]), Some(10));
        assert_eq!(signed_end(&[]), None);
        assert_eq!(signed_end(&[0, 1, 2]), None);
        assert_eq!(signed_end(&[u64::MAX, 1]), None);
    }
}
