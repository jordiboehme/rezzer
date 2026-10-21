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

use std::collections::{HashMap, HashSet};

use pdfparts::{FieldKind, Form, Issue, IssueKind, Link, LinkTarget, Pdf, Script, Sig, SigKind};
use rezzer_model::{
    Anchor, Edge, Embedded, FieldAction, FormField, Formula, Graph, LossReason, Node, Role,
    Severity, Signature, Value,
};

use crate::Out;
use crate::objects::{History, Kept, iso, quote};

/// How many characters of a format script stand for the format when there are no words for it.
const FORMAT_CHARS: usize = 80;

/// The note of a calculation that builds field names at run time.
const DYNAMIC: &str = "reads unknown fields, see script";

/// Reads the form logic of `pdf` into `out`. `links` are the file's links, which the link list
/// names already; `history` says where the saves of the file end.
pub(crate) fn read(pdf: &Pdf, links: &[Link], history: &History, kept: &Kept<'_>, out: &mut Out) {
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
    signatures(sigs, history, kept, out);
    if let Some(form) = form {
        fields(form, scripts, kept, out);
    }
    list_scripts(scripts, links, kept, out);
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
    // Approval and certification signatures sign the content; a usage rights signature only
    // unlocks features of a viewer, so it is listed but does not make the file signed.
    let signs = sigs.iter().any(|s| s.kind != SigKind::UsageRights);
    if signs || !sigs_cut {
        facts.signed = Some(signs);
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

/// How many of the kept saves came after signed bytes that end at `end`: a save is later when
/// the save before it ends at or after `end`, so the line end after `%%EOF`, signed or not,
/// counts with the save it ends. The oldest kept save is later only when older saves were
/// skipped, and then not known: the second value says whether the count is only a lower bound.
fn saves_after(end: u64, history: &History) -> (usize, bool) {
    let ends = &history.ends;
    let later = ends.windows(2).filter(|w| w[0] >= end).count();
    let bound = history.skipped > 0 && ends.first().is_none_or(|&first| first >= end);
    (later, bound)
}

/// One model signature per signature, with the saves after its signed bytes. Past the
/// revisions that are compared, the count is a lower bound, which one loss says.
fn signatures(sigs: &[Sig], history: &History, kept: &Kept<'_>, out: &mut Out) {
    let mut bounds: Vec<usize> = Vec::new();
    for (k, s) in sigs.iter().enumerate() {
        let (later, bound) =
            signed_end(&s.byte_range).map_or((0, false), |end| saves_after(end, history));
        if bound {
            bounds.push(k + 1);
        }
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
            break;
        }
        out.doc.facets.signatures.push(model);
    }
    if let Some(first) = bounds.first() {
        let n = bounds.len();
        out.loss(
            "signatures",
            format!(
                "the saves after {} are counted only among the newest {} saves, so there may \
                 be more: at least that many came after {}",
                if n == 1 {
                    format!("signature {first}")
                } else {
                    format!("{n} signatures (the first is signature {first})")
                },
                history.ends.len(),
                if n == 1 { "it" } else { "each" }
            ),
            LossReason::Limit,
            Severity::Info,
        );
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
fn fields(form: &Form, scripts: &[Script], kept: &Kept<'_>, out: &mut Out) {
    // The field actions of every kind, also those whose script was not read (too long, or no
    // text), by their trigger `field <full name> <event>`.
    let triggers: HashSet<&str> = scripts
        .iter()
        .filter(|s| s.trigger.starts_with("field "))
        .map(|s| s.trigger.as_str())
        .collect();
    let mut calc: HashMap<&str, u32> = HashMap::new();
    for (k, name) in form.calc_order.iter().enumerate() {
        calc.entry(name.as_str())
            .or_insert(u32::try_from(k + 1).unwrap_or(u32::MAX));
    }
    let total = form.fields.len();
    let mut dropped = Dropped::default();
    // Calculations kept as a field action but not as a formula.
    let mut calcs_dropped = Dropped::default();
    // The kept fields' names in model order, each with the names its calculation reads.
    let mut kept_fields: Vec<(&str, Vec<String>)> = Vec::new();
    for (k, f) in form.fields.iter().enumerate() {
        let name = f.full_name.as_str();
        let order = calc.get(name).copied();
        let calculates = f.actions.iter().any(|(event, _)| event == "calculate")
            || triggers.contains(format!("field {name} calculate").as_str());
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
                    calcs_dropped.add(name, event, why);
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
    if let Some((field, _)) = calcs_dropped.first {
        let n = calcs_dropped.count;
        out.loss(
            "form",
            format!(
                "{n} {} not kept as formulas, the first that of field {field}; the script \
                 stays with its field: {}",
                if n == 1 {
                    "calculation is"
                } else {
                    "calculations are"
                },
                calcs_dropped.why
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
    let mut seen = HashSet::new();
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

/// The action word pdfparts gives an action type, for the types the links list by name.
fn action_word(name: &str) -> String {
    match name {
        "GoToR" => "goto-remote".into(),
        "GoToE" => "goto-embedded".into(),
        "GoTo3DView" => "goto-3d-view".into(),
        "SetOCGState" => "set-layer-state".into(),
        "URI" => "uri".into(),
        _ => {
            let mut out = String::new();
            for (i, c) in name.chars().enumerate() {
                if c.is_uppercase() && i > 0 {
                    out.push('-');
                }
                out.extend(c.to_lowercase());
            }
            out
        }
    }
}

/// A link action: its page, action word and file.
type LinkKey = (u32, String, Option<String>);

/// The first action of each link that the links list names, by page and action word, with its
/// file for a Launch or remote link, each with how many links name it.
fn listed_by_links(links: &[Link]) -> HashMap<LinkKey, usize> {
    let mut out = HashMap::new();
    for l in links {
        let key = match &l.target {
            LinkTarget::Launch(file) => (l.page, "launch".to_string(), Some(file.to_string())),
            LinkTarget::Remote { file, .. } => {
                (l.page, "goto-remote".to_string(), Some(file.to_string()))
            }
            // A script link is listed here, with its text.
            LinkTarget::Other(action) if &**action == "JavaScript" => continue,
            LinkTarget::Other(action) => (l.page, action_word(action), None),
            _ => continue,
        };
        *out.entry(key).or_insert(0) += 1;
    }
    out
}

/// Every script and action of the file as an embedded `script`, with its text: the trigger
/// as the name, the action's kind and the script's name as the detail. The first action of a
/// link that the links list names already is left out; actions later in a link's chain are
/// listed here. A JavaScript action without a readable script is listed without a text, and
/// one loss says so. A text that does not fit is left out of its entry; an entry that does
/// not fit ends the list.
fn list_scripts(scripts: &[Script], links: &[Link], kept: &Kept<'_>, out: &mut Out) {
    let mut by_links = listed_by_links(links);
    let mut take = |key: LinkKey| match by_links.get_mut(&key) {
        Some(n) if *n > 0 => {
            *n -= 1;
            true
        }
        _ => false,
    };
    let listed: Vec<&Script> = scripts
        .iter()
        .filter(|s| {
            let Some(page) = s
                .trigger
                .strip_prefix("link page ")
                .and_then(|p| p.parse::<u32>().ok())
            else {
                return true;
            };
            if s.kind == "javascript" {
                return true;
            }
            let target = s.target.as_deref().map(str::to_string);
            let named = match s.kind.as_str() {
                "launch" | "goto-remote" => take((page, s.kind.clone(), target)),
                other => take((page, other.to_string(), None)),
            };
            !named
        })
        .collect();
    let unread: Vec<&&Script> = listed
        .iter()
        .filter(|s| s.kind == "javascript" && s.text.is_none())
        .collect();
    if let Some(first) = unread.first() {
        let n = unread.len();
        out.loss(
            "scripts",
            format!(
                "{n} JavaScript {} listed without a script, which could not be read (the first is \
                 the {} one); a limit or damage loss may say why",
                if n == 1 { "action is" } else { "actions are" },
                quote(&first.trigger)
            ),
            LossReason::Uncertain,
            Severity::Warning,
        );
    }
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
                "{}: {why}",
                if dropped == 1 {
                    format!("1 script is kept without its text ({})", quote(&trigger))
                } else {
                    format!(
                        "{dropped} scripts are kept without their text, the first {}",
                        quote(&trigger)
                    )
                }
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
