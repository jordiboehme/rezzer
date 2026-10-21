// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The Info dictionary and XMP as properties, and the facts of the file as a whole. Every copy
//! is charged like the rest of the object layer ([`Kept`]).

use std::cell::Cell as Counter;
use std::collections::HashSet;

use pdfparts::objects::xmp_date;
use pdfparts::{Info, IssueKind, Pdf};
use rezzer_model::{LossReason, Severity, Value};

use crate::Out;
use crate::objects::{Kept, count_is, given, is_are, iso, quote};

/// The facts of the file as a whole that need no copy.
pub(crate) fn facts(pdf: &Pdf, out: &mut Out) {
    let facts = &mut out.doc.properties.facts;
    facts.pages = Some(pdf.page_count());
    facts.tagged = Some(pdf.tagged());
    facts.encrypted = Some(pdf.encryption().is_some());
}

/// Properties from the Info dictionary, else from XMP; an XMP value that differs from the Info
/// value it stands beside is kept as `xmp_<name>`. Then the language, version, PDF/A and
/// PDF/UA parts and the layers.
pub(crate) fn properties(pdf: &Pdf, kept: &Kept<'_>, out: &mut Out) {
    let info: Info = pdf.info();
    let mut xmp = pdf.xmp();
    // An XMP date is used only when it is a real date; else it is said and left out.
    xmp.retain(|(key, value)| {
        if !matches!(key.as_str(), "xmp:CreateDate" | "xmp:ModifyDate") || xmp_date(value).is_some()
        {
            return true;
        }
        out.loss(
            "metadata",
            format!("the XMP {key} is not a date: {}", quote(value)),
            LossReason::Damaged,
            Severity::Warning,
        );
        false
    });
    let x = |key: &str| given(xmp.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()));
    // A date in the Info dictionary that does not parse, with no XMP date to stand in.
    for (name, d, key) in [
        ("creation", info.created.as_ref(), "xmp:CreateDate"),
        ("modification", info.modified.as_ref(), "xmp:ModifyDate"),
    ] {
        if let Some(d) = d
            && d.iso.is_none()
            && x(key).is_none()
        {
            out.loss(
                "metadata",
                format!(
                    "the {name} date of the document information is not a PDF date: {}",
                    quote(&d.raw)
                ),
                LossReason::Unsupported,
                Severity::Info,
            );
        }
    }
    let mut left_out: Vec<&str> = Vec::new();
    let mut why: Option<String> = None;
    let props = &mut out.doc.properties;
    // The Info value, else the XMP value at `key`; and the XMP value when it differs.
    let pick = |info: Option<&str>, key: &str| -> (Option<String>, Option<String>) {
        let info = given(info);
        let xmp = x(key);
        match (info, xmp) {
            (Some(i), Some(v)) if i != v => (Some(i.to_string()), Some(v.to_string())),
            (Some(i), _) => (Some(i.to_string()), None),
            (None, v) => (v.map(str::to_string), None),
        }
    };
    let mut keep = |name: &'static str, texts: &[&str]| -> bool {
        match kept.take(1, texts) {
            Ok(()) => true,
            Err(w) => {
                why.get_or_insert(w);
                left_out.push(name);
                false
            }
        }
    };
    let mut custom: Vec<(&str, String)> = Vec::new();
    let (title, xmp_title) = pick(info.title.as_deref(), "dc:title");
    if let Some(t) = title
        && keep("title", &[&t])
    {
        props.title = Some(t);
    }
    // Lists are copied one item at a time, each charged as it is copied.
    let mut lists_cut: Vec<(&str, (usize, usize, String))> = Vec::new();
    // The Info author is one name; the XMP creators are a list, joined with "; ".
    let (author, xmp_author) = pick(info.author.as_deref(), "dc:creator");
    let (keywords, xmp_keywords) = pick(info.keywords.as_deref(), "pdf:Keywords");
    let dates = [
        ("created", iso(info.created.as_ref()), "xmp:CreateDate"),
        ("modified", iso(info.modified.as_ref()), "xmp:ModifyDate"),
    ];
    for (name, from_info, key) in dates {
        let (date, xmp_date) = pick(from_info, key);
        if let Some(d) = date
            && keep(name, &[&d])
        {
            let value = Some(Value::DateTime { value: d });
            match name {
                "created" => props.created = value,
                _ => props.modified = value,
            }
        }
        if let Some(v) = xmp_date {
            custom.push((
                if name == "created" {
                    "xmp_create_date"
                } else {
                    "xmp_modify_date"
                },
                v,
            ));
        }
    }
    let pairs = [
        (
            info.subject.as_deref(),
            "dc:description",
            "subject",
            "xmp_description",
        ),
        (
            info.creator.as_deref(),
            "xmp:CreatorTool",
            "creator",
            "xmp_creator_tool",
        ),
        (
            info.producer.as_deref(),
            "pdf:Producer",
            "producer",
            "xmp_producer",
        ),
    ];
    for (from_info, key, name, xmp_name) in pairs {
        let (v, xv) = pick(from_info, key);
        if let Some(v) = v {
            custom.push((name, v));
        }
        if let Some(v) = xv {
            custom.push((xmp_name, v));
        }
    }
    for (name, v) in [
        ("xmp_title", xmp_title),
        ("xmp_creator", xmp_author),
        ("xmp_keywords", xmp_keywords),
    ] {
        if let Some(v) = v {
            custom.push((name, v));
        }
    }
    if let Some(lang) = pdf.language()
        && keep("language", &[&lang])
    {
        props.language = Some(lang);
    }
    custom.push(("pdf_version", pdf.version()));
    if let Some(part) = x("pdfaid:part") {
        let conformance = x("pdfaid:conformance").unwrap_or_default().to_lowercase();
        custom.push(("pdfa", format!("PDF/A-{part}{conformance}")));
    }
    if let Some(part) = x("pdfuaid:part") {
        custom.push(("pdfua", format!("PDF/UA-{part}")));
    }
    for (name, value) in custom {
        if keep(name, &[name, &value]) {
            props.custom.insert(name.to_string(), Value::Text { value });
        }
    }
    // The lists come after every single value, so a long list never costs one of them, and
    // together they take at most half of the object layer's share.
    let lists = Counter::new(kept.share() / 2);
    if let Some(a) = author {
        let names: Box<dyn Iterator<Item = &str>> = if given(info.author.as_deref()).is_some() {
            Box::new(std::iter::once(a.as_str()))
        } else {
            Box::new(a.split("; ").map(str::trim).filter(|n| !n.is_empty()))
        };
        if let Some(cut) = copy_list(names, false, kept, &lists, &mut props.authors) {
            lists_cut.push(("authors", cut));
        }
    }
    if let Some(k) = keywords {
        // Some producers write the whole list in quotes.
        let list = k
            .strip_prefix('"')
            .and_then(|k| k.strip_suffix('"'))
            .unwrap_or(&k);
        let words = list
            .split([',', ';'])
            .map(str::trim)
            .filter(|w| !w.is_empty());
        if let Some(cut) = copy_list(words, true, kept, &lists, &mut props.keywords) {
            lists_cut.push(("keywords", cut));
        }
    }
    // The layers, each charged as it is added to the list.
    let layers = pdf.layers();
    if !layers.is_empty() {
        // A count pdfparts cut is not the file's count: then the fact is left out.
        let cut_short = pdf
            .issues()
            .iter()
            .any(|i| i.location == "layers" && i.kind == IssueKind::Limit);
        if !cut_short {
            props.facts.layers = Some(u32::try_from(layers.len()).unwrap_or(u32::MAX));
        }
        let mut list = String::new();
        for (k, l) in layers.iter().enumerate() {
            let name = given(Some(&l.name)).unwrap_or("unnamed layer");
            let shown = if l.visible { "shown" } else { "hidden" };
            let item = format!("{name} ({shown})");
            if let Err(why) = kept.take(1, &[&item]) {
                lists_cut.push(("layers", (k + 1, layers.len(), why)));
                break;
            }
            if !list.is_empty() {
                list.push_str(", ");
            }
            list.push_str(&item);
        }
        if !list.is_empty() {
            props
                .custom
                .insert("layers".into(), Value::Text { value: list });
        }
    }
    for (what, (first, total, why)) in lists_cut {
        let (one, verb) = match what {
            "layers" => ("layer", "listed"),
            "authors" => ("author", "kept"),
            _ => ("keyword", "kept"),
        };
        let left = total - first + 1;
        out.loss(
            "metadata",
            format!(
                "{} not {verb}, from {one} {first} on: {why}",
                count_is(left, one, what)
            ),
            LossReason::Limit,
            Severity::Warning,
        );
    }
    if let Some(why) = why {
        out.loss(
            "metadata",
            format!(
                "{} {} not kept: {why}",
                left_out.join(", "),
                is_are(left_out.len())
            ),
            LossReason::Limit,
            Severity::Warning,
        );
    }
}

/// Copies the items of `items` into `into`, each charged as it is copied, repeats left out
/// when `dedup`. Returns the cut, if one: the number of the first item not kept, how many
/// items there are and why.
fn copy_list<'w>(
    items: impl Iterator<Item = &'w str>,
    dedup: bool,
    kept: &Kept<'_>,
    cap: &Counter<u64>,
    into: &mut Vec<String>,
) -> Option<(usize, usize, String)> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut cut: Option<(usize, String)> = None;
    let mut n = 0;
    for item in items {
        n += 1;
        if cut.is_some() || (dedup && seen.contains(item)) {
            continue;
        }
        if let Err(why) = kept.take_within(cap, 1, &[item]) {
            cut = Some((n, why));
            continue;
        }
        if dedup {
            seen.insert(item);
        }
        into.push(item.to_string());
    }
    cut.map(|(first, why)| (first, n, why))
}
